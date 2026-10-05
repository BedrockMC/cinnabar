package authcache

import (
	"context"
	"encoding/hex"
	"errors"
	"io"
	"sync"
	"time"

	"github.com/df-mc/go-playfab/v2"
	"github.com/df-mc/go-xsapi/v2"
	"github.com/df-mc/go-xsapi/v2/xal/nsal"
	"github.com/google/uuid"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"golang.org/x/oauth2"
)

const (
	serviceRefreshLead = 10 * time.Minute // replace the service token this long before it expires
	refreshAttempt     = 30 * time.Second // bounds one background exchange so a hung request cannot wedge refresh
	refreshRecheck     = 15 * time.Minute // longest sleep, so suspend or another process's refresh is noticed
	refreshRetryMin    = time.Minute
	refreshRetryMax    = 15 * time.Minute
)

// serviceDeviceNamespace scopes the per-account Minecraft service device ID.
var serviceDeviceNamespace = uuid.MustParse("52194710-c463-4fa4-98a9-334c1b905e01")

// CompleteSignIn exchanges the signed-in account's service token and persists it next to the Microsoft
// token at oauthPath, so a join only mints its key-bound token.
func CompleteSignIn(ctx context.Context, oauthPath string, oauth oauth2.TokenSource, diagnostics io.Writer) error {
	return completeSignIn(ctx, oauthPath, oauth, diagnostics, defaultDerivedDeps())
}

func completeSignIn(ctx context.Context, oauthPath string, oauth oauth2.TokenSource, diagnostics io.Writer, deps derivedDeps) error {
	account := newAccount(ctx, DerivedCachePath(oauthPath), oauth, diagnostics, deps)
	if account == nil {
		return errors.New("authentication: no signed-in account")
	}
	defer func() { _ = account.Close() }()
	_, err := account.ServiceToken(ctx)
	return err
}

// KeepFresh refreshes the cached service token shortly before it expires while signed in. It returns
// when ctx ends or the account closes, and at once when another KeepFresh already serves this account.
func (s *Account) KeepFresh(ctx context.Context) {
	if !s.refreshing.CompareAndSwap(false, true) {
		return
	}
	defer s.refreshing.Store(false)
	retry := refreshRetryMin
	for {
		attempt, cancel := context.WithTimeout(ctx, refreshAttempt)
		remaining, err := s.refreshServiceAhead(attempt, serviceRefreshLead)
		cancel()
		var wait time.Duration
		switch {
		case ctx.Err() != nil || s.Closed() || errors.Is(err, ErrAccountClosed) || errors.Is(err, errAccountChanged):
			return
		case err != nil:
			wait, retry = retry, min(retry*2, refreshRetryMax)
		default:
			wait, retry = remaining-serviceRefreshLead, refreshRetryMin
		}
		timer := time.NewTimer(min(max(wait, refreshRetryMin), refreshRecheck))
		select {
		case <-ctx.Done():
		case <-s.ctx.Done():
		case <-timer.C:
			continue
		}
		timer.Stop()
		return
	}
}

// refreshServiceAhead replaces the service token once it is within lead of expiry and returns how long
// the current token remains valid, on the service clock its validity uses. While the current token is
// still valid the exchange runs outside the account gate and cache lease, so joins keep using it; a
// failed exchange keeps it. A token another process already refreshed is reused through the shared cache.
func (s *Account) refreshServiceAhead(ctx context.Context, lead time.Duration) (time.Duration, error) {
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	plan, remaining, err := s.planServiceRefresh(ctx, lead)
	if plan == nil {
		return remaining, err
	}
	if !s.exchanging.CompareAndSwap(false, true) {
		return remaining, nil
	}
	defer s.exchanging.Store(false)
	tickets := &playFabTickets{account: s, plan: plan, client: plan.client}
	source := s.deps.services(plan.environment, tickets, nil, plan.deviceID)
	token, err := source.ServiceToken(ctx)
	if err != nil || token == nil || !token.Valid() {
		tickets.discard()
		if ctx.Err() != nil {
			return remaining, ctx.Err()
		}
		return remaining, errors.New("authentication: refresh service credential")
	}
	return s.installServiceRefresh(ctx, plan, tickets, source, token)
}

// serviceRefreshPlan captures what an early exchange needs so it can run without the account gate.
type serviceRefreshPlan struct {
	binding     string
	before      *service.Token
	environment *service.AuthorizationEnvironment
	signer      xsapi.TokenAndSignaturer
	client      *playfab.Client // nil when the exchange must log in first
	deviceID    string
	session     string
}

// planServiceRefresh returns nil with the remaining lifetime when no early exchange is due; an
// already invalid token is refreshed in place, as a join would need it anyway.
func (s *Account) planServiceRefresh(ctx context.Context, lead time.Duration) (*serviceRefreshPlan, time.Duration, error) {
	if err := s.lock(ctx); err != nil {
		return nil, 0, err
	}
	defer s.unlock()
	if _, err := s.tokenLocked(ctx); err != nil {
		return nil, 0, err
	}
	lease, err := s.acquireLeaseLocked(ctx)
	if err != nil {
		return nil, 0, err
	}
	if lease != nil {
		defer lease.Close()
		s.reloadLocked()
	}
	if s.service == nil || !s.service.Valid() {
		token, err := s.serviceTokenLocked(ctx, lease != nil, true)
		if err != nil {
			return nil, 0, err
		}
		return nil, token.Remaining(), nil
	}
	if s.service.Remaining() > lead {
		return nil, s.service.Remaining(), nil
	}
	if err := s.ensureEnvironmentLocked(ctx); err != nil {
		return nil, s.service.Remaining(), err
	}
	return &serviceRefreshPlan{
		binding:     s.binding,
		before:      s.service,
		environment: s.environment,
		signer:      nsal.NewResolver(s.session),
		client:      s.playfab,
		deviceID:    s.serviceDeviceIDLocked(),
		session:     sessionFingerprint(s.session.Snapshot()),
	}, s.service.Remaining(), nil
}

// installServiceRefresh swaps in an exchanged token unless the account changed or a newer token won.
func (s *Account) installServiceRefresh(
	ctx context.Context,
	plan *serviceRefreshPlan,
	tickets *playFabTickets,
	source service.TokenSource,
	token *service.Token,
) (time.Duration, error) {
	if err := s.lock(ctx); err != nil {
		tickets.discard()
		return 0, err
	}
	defer s.unlock()
	if client := tickets.loggedIn(); client != nil {
		if s.playfab == nil && s.binding == plan.binding {
			context.AfterFunc(s.ctx, func() { _ = client.Close() })
			s.playfab = client
		} else {
			_ = client.Close()
		}
	}
	lease, err := s.acquireLeaseLocked(ctx)
	if err != nil {
		return 0, err
	}
	if lease != nil {
		defer lease.Close()
		s.reloadLocked()
	}
	current := s.service
	if s.binding != plan.binding || s.environment != plan.environment ||
		(current != nil && current != plan.before && current.ValidUntil.After(token.ValidUntil)) {
		if current != nil && current.Valid() {
			return current.Remaining(), nil
		}
		return 0, errors.New("authentication: account changed during service refresh")
	}
	s.diagnostic("refresh", "service", "expiring")
	s.service, s.services = token, source
	if plan.session != sessionFingerprint(s.session.Snapshot()) {
		s.updateOAuthBindingLocked(ctx)
	}
	s.persistLocked(ctx, lease != nil)
	return token.Remaining(), nil
}

// playFabTickets serves session tickets outside the account gate, logging in on first need when the
// account has no PlayFab session yet.
type playFabTickets struct {
	account *Account
	plan    *serviceRefreshPlan
	mu      sync.Mutex
	client  *playfab.Client
	created bool
}

func (t *playFabTickets) SessionTicket(ctx context.Context) (string, error) {
	t.mu.Lock()
	defer t.mu.Unlock()
	if t.client == nil {
		client, err := t.account.deps.login(ctx, t.plan.environment, t.plan.signer)
		if err != nil {
			return "", errors.New("authentication: PlayFab login")
		}
		t.client, t.created = client, true
	}
	return t.client.SessionTicket(ctx)
}

// loggedIn returns a PlayFab session this exchange created, for the account to adopt.
func (t *playFabTickets) loggedIn() *playfab.Client {
	t.mu.Lock()
	defer t.mu.Unlock()
	if !t.created {
		return nil
	}
	return t.client
}

func (t *playFabTickets) discard() {
	if client := t.loggedIn(); client != nil {
		_ = client.Close()
	}
}

// serviceDeviceIDLocked returns the account's stable, undashed service device ID derived from its XUID,
// as an install keeps one device ID across restarts; "" before any token carries the XUID.
func (s *Account) serviceDeviceIDLocked() string {
	snapshot := s.session.Snapshot()
	if snapshot == nil {
		return ""
	}
	for _, token := range snapshot.XSTSTokens {
		if token == nil {
			continue
		}
		for _, info := range token.DisplayClaims.UserInfo {
			if info.XUID != "" {
				id := uuid.NewSHA1(serviceDeviceNamespace, []byte(info.XUID))
				return hex.EncodeToString(id[:])
			}
		}
	}
	return ""
}
