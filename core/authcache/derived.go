package authcache

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/sha256"
	"crypto/x509"
	_ "embed"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"maps"
	"net/url"
	"path/filepath"
	"sync"
	"sync/atomic"
	"time"

	"github.com/df-mc/go-playfab/v2"
	"github.com/df-mc/go-playfab/v2/title"
	"github.com/df-mc/go-xsapi/v2"
	"github.com/df-mc/go-xsapi/v2/xal/nsal"
	"github.com/df-mc/go-xsapi/v2/xal/sisu"
	"github.com/df-mc/go-xsapi/v2/xal/xasd"
	"github.com/df-mc/go-xsapi/v2/xal/xsts"
	"github.com/google/uuid"
	"github.com/hashimthearab/rust-mcbe/core/clientplatform"
	"github.com/hashimthearab/rust-mcbe/core/internal/lockfile"
	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/auth"
	"github.com/sandertv/gophertunnel/minecraft/protocol"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"golang.org/x/oauth2"
)

const (
	derivedCacheVersion = 1
)

//go:embed derived_suffix.txt
var derivedCacheSuffix string

// DerivedCachePath returns the private cache path used for authentication
// state derived from the Microsoft token at oauthPath.
func DerivedCachePath(oauthPath string) string {
	return oauthPath + derivedCacheSuffix
}

// NewAccount returns the signed-in account's runtime: the Xbox, PlayFab and Minecraft service
// credentials every consumer shares, with proof-key-bound state persisted at path (empty disables
// persistence). The OAuth source stays authoritative: new token material makes the cache miss.
// Cache failures are optional misses reported without paths or secrets. Close ends the runtime.
func NewAccount(ctx context.Context, path string, oauth oauth2.TokenSource, diagnostics io.Writer) *Account {
	return newAccount(ctx, path, oauth, diagnostics, defaultDerivedDeps())
}

type derivedDeps struct {
	discover func(context.Context) (*service.AuthorizationEnvironment, error)
	login    func(context.Context, *service.AuthorizationEnvironment, xsapi.TokenAndSignaturer) (*playfab.Client, error)
	services func(env *service.AuthorizationEnvironment, tickets service.SessionTicketSource, token *service.Token, deviceID, sessionID string) service.TokenSource
	mint     func(context.Context, *service.AuthorizationEnvironment, service.TokenSource, *ecdsa.PublicKey) (string, error)
}

func defaultDerivedDeps() derivedDeps {
	return derivedDeps{
		discover: func(ctx context.Context) (*service.AuthorizationEnvironment, error) {
			discovery, err := service.Default(ctx)
			if err != nil {
				return nil, err
			}
			env := new(service.AuthorizationEnvironment)
			if err := discovery.Environment(env); err != nil {
				return nil, err
			}
			env.HTTPClient = authHTTPClient
			return env, nil
		},
		login: func(ctx context.Context, env *service.AuthorizationEnvironment, signer xsapi.TokenAndSignaturer) (*playfab.Client, error) {
			return playfab.LoginWithXbox(ctx, env.PlayFabTitleID, signer, playfab.ClientConfig{CreateAccount: true, HTTPClient: authHTTPClient})
		},
		services: func(env *service.AuthorizationEnvironment, tickets service.SessionTicketSource, token *service.Token, deviceID, sessionID string) service.TokenSource {
			config := clientplatform.TokenConfig()
			config.Device.ID = deviceID
			config.SessionID = sessionID
			return env.ResumeTokenSource(tickets, config, token)
		},
		mint: func(ctx context.Context, env *service.AuthorizationEnvironment, source service.TokenSource, key *ecdsa.PublicKey) (string, error) {
			return minecraft.NewMultiplayerTokenSource(env, source).MultiplayerToken(ctx, key)
		},
	}
}

type derivedEnvironment struct {
	ServiceURI     string      `json:"service_uri"`
	Issuer         string      `json:"issuer"`
	PlayFabTitleID title.Title `json:"playfab_title_id"`
}

type derivedState struct {
	Version       int                 `json:"version"`
	OAuthBinding  string              `json:"oauth_binding"`
	ClientBinding string              `json:"client_binding"`
	Environment   *derivedEnvironment `json:"environment,omitempty"`
	DeviceToken   *xasd.Token         `json:"device_token,omitempty"`
	ProofKey      string              `json:"proof_key,omitempty"`
	SISU          *sisu.Snapshot      `json:"sisu,omitempty"`
	ServiceToken  *service.Token      `json:"service_token,omitempty"`
}

// Account is the per-account runtime; every method is safe for concurrent use. The gate guards
// state only briefly and is never held across a network request: each credential is derived in its
// own flight, so one hung request never delays an unrelated credential.
type Account struct {
	ctx           context.Context
	cancel        context.CancelFunc
	gate          chan struct{}
	oauthGate     chan struct{} // orders OAuth reads with the binding change each applies
	path          string
	diagnostics   io.Writer
	diagnosticsMu sync.Mutex // diagnostics are written outside the gate
	oauth         oauth2.TokenSource
	binding       string
	client        string
	device        xasd.TokenSource
	deviceToken   *xasd.Token
	session       *accountSession
	xstsTokens    map[string]*xsts.Token // last XSTS token per relying party restored or returned
	environment   *service.AuthorizationEnvironment
	cachedEnv     *derivedEnvironment
	service       *service.Token
	services      service.TokenSource // native source seeded with service; rebuilt after every restore
	sessionID     string              // Session-Id every service request names for this launch
	playfab       *playfab.Client     // logged in on first need; closed only by Close
	resolver      *nsal.Resolver      // PlayFab's endpoint resolver; keeps NSAL title data for the account's life
	closed        atomic.Bool
	refreshing    atomic.Bool            // one KeepFresh per account
	exchanging    atomic.Bool            // one early service exchange per account
	persisted     string                 // fingerprint of the bundle bytes last read or written
	rejected      map[string]*xsts.Token // XSTS tokens a relying party refused; re-evicted after every reload
	deps          derivedDeps

	flightMu sync.Mutex
	flights  map[string]*flight

	activeMu sync.Mutex
	active   int
	closing  bool
	idle     chan struct{}
}

var (
	_ oauth2.TokenSource               = (*Account)(nil)
	_ xsapi.TokenSource                = (*Account)(nil)
	_ minecraft.MultiplayerTokenSource = (*Account)(nil)
	_ nsal.TokenSource                 = (*Account)(nil)
	_ nsal.TokenInvalidator            = (*Account)(nil)
	_ service.TokenSource              = (*Account)(nil)
	_ service.TokenInvalidator         = (*Account)(nil)
	_ service.SessionIdentifier        = (*Account)(nil)
)

// ErrAccountClosed is returned once the account has been signed out or shut down.
var ErrAccountClosed = errors.New("authentication: account is closed")

func newAccount(ctx context.Context, path string, oauth oauth2.TokenSource, diagnostics io.Writer, deps derivedDeps) *Account {
	if oauth == nil {
		return nil
	}
	if diagnostics == nil {
		diagnostics = io.Discard
	}
	ctx, cancel := context.WithCancel(ctx)
	source := &Account{
		ctx: ctx, cancel: cancel, gate: make(chan struct{}, 1), oauthGate: make(chan struct{}, 1), diagnostics: diagnostics,
		oauth: oauth, client: clientBinding(), sessionID: uuid.NewString(), deps: deps, xstsTokens: make(map[string]*xsts.Token),
	}
	source.resolver = nsal.NewResolver(source)
	defer func() {
		if source.session == nil {
			source.device = xasd.ReuseTokenSource(auth.AndroidConfig.Config.Config, nil, nil)
			source.session = newAccountSession(source, &sisu.SessionConfig{DeviceTokenSource: source.device})
		}
	}()
	tok, err := source.oauthToken(ctx)
	if err != nil || tok == nil {
		return source
	}
	source.binding = oauthBinding(tok)
	if path == "" {
		return source
	}
	path, err = filepath.Abs(path)
	if err != nil {
		return source
	}
	source.path = path
	binding, client := source.binding, source.client
	state, fingerprint, err := loadDerivedBundle(source.path)
	switch {
	case err == nil && state.OAuthBinding == binding && state.ClientBinding == client:
		if err := source.restore(state); err != nil {
			source.diagnostic("miss", "bundle", "invalid")
		} else {
			source.persisted = fingerprint
			source.diagnostic("hit", "bundle", "bound")
		}
	case err == nil:
		source.diagnostic("miss", "bundle", "binding")
		source.resetLocked(binding)
	case errors.Is(err, fs.ErrNotExist):
		source.diagnostic("miss", "bundle", "missing")
	case errors.Is(err, errDerivedCacheMiss):
		source.diagnostic("miss", "bundle", "invalid")
	default:
		source.diagnostic("miss", "bundle", "unsafe")
	}
	return source
}

func (s *Account) diagnostic(event, layer, reason string) {
	s.diagnosticsMu.Lock()
	defer s.diagnosticsMu.Unlock()
	_, _ = fmt.Fprintf(s.diagnostics, "AUTH_ACCEL_CACHE event=%s layer=%s reason=%s\n", event, layer, reason)
}

// Token returns the OAuth credential only while this account is open.
func (s *Account) Token() (*oauth2.Token, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	return s.currentOAuth(s.ctx, false)
}

func (s *Account) DeviceToken(ctx context.Context) (*xasd.Token, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if err := s.prepare(ctx); err != nil {
		return nil, err
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	token := s.deviceToken
	s.unlock()
	if token.Valid() {
		s.diagnostic("reuse", "device", "valid")
		return token, nil
	}
	return awaitFlight(s, ctx, "device", s.deriveDevice)
}

func (s *Account) deriveDevice(ctx context.Context) (*xasd.Token, error) {
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	device, before := s.device, s.deviceToken
	s.unlock()
	token, err := device.DeviceToken(ctx)
	if err != nil {
		return nil, err
	}
	if before != nil && before.Token == token.Token {
		s.diagnostic("reuse", "device", "valid")
		return token, nil
	}
	s.diagnostic("refresh", "device", "expired")
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	changed := s.device == device && s.deviceToken != token
	if changed {
		s.deviceToken = token
	}
	s.unlock()
	if changed {
		s.publish(ctx)
	}
	return token, nil
}

func (s *Account) ProofKey() *ecdsa.PrivateKey {
	if err := s.lock(s.ctx); err != nil {
		return nil
	}
	defer s.unlock()
	if s.closed.Load() {
		return nil
	}
	return s.device.ProofKey()
}

// XSTSToken returns the account's token for relyingParty; each relying party refreshes in its own
// flight, so a join's token never waits on another party's request.
func (s *Account) XSTSToken(ctx context.Context, relyingParty string) (*xsts.Token, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if err := s.prepare(ctx); err != nil {
		return nil, err
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	token := s.xstsTokens[relyingParty]
	s.unlock()
	if token.Valid() {
		s.diagnostic("reuse", "xsts", "valid")
		return token, nil
	}
	return awaitFlight(s, ctx, "xsts "+relyingParty, func(ctx context.Context) (*xsts.Token, error) {
		return s.deriveXSTS(ctx, relyingParty)
	})
}

func (s *Account) deriveXSTS(ctx context.Context, relyingParty string) (*xsts.Token, error) {
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	session, device := s.session, s.device
	s.unlock()
	token, err := session.XSTSToken(ctx, relyingParty)
	if err != nil {
		return nil, err
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	rejected := s.rejectedLocked(relyingParty, token)
	s.unlock()
	if rejected {
		// The request overlapped an invalidation and read the refused token before SISU evicted it.
		session.InvalidateXSTSToken(relyingParty, token)
		if token, err = session.XSTSToken(ctx, relyingParty); err != nil {
			return nil, err
		}
	}
	deviceToken, deviceErr := device.DeviceToken(ctx)
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	changed := false
	if s.session == session && !s.rejectedLocked(relyingParty, token) {
		changed = s.xstsTokens[relyingParty] != token
		s.xstsTokens[relyingParty] = token
		if deviceErr == nil && s.device == device && s.deviceToken != deviceToken {
			s.deviceToken, changed = deviceToken, true
		}
	}
	s.unlock()
	if !changed {
		s.diagnostic("reuse", "xsts", "valid")
		return token, nil
	}
	s.diagnostic("refresh", "xsts", "expired")
	s.publish(ctx)
	return token, nil
}

func (s *Account) rejectedLocked(relyingParty string, token *xsts.Token) bool {
	rejected := s.rejected[relyingParty]
	return rejected != nil && token != nil && rejected.Token == token.Token
}

// InvalidateXSTSToken evicts rejected through SISU and persists the eviction so no reload, in this
// process or another, can resurrect it.
func (s *Account) InvalidateXSTSToken(relyingParty string, rejected *xsts.Token) {
	if rejected == nil || rejected.Token == "" {
		return
	}
	if s.begin() != nil {
		return
	}
	defer s.end()
	ctx, cancel := context.WithTimeout(s.ctx, 10*time.Second)
	defer cancel()
	if err := s.lock(ctx); err != nil {
		return
	}
	if s.rejected == nil {
		s.rejected = make(map[string]*xsts.Token)
	}
	s.rejected[relyingParty] = rejected
	s.unlock()
	if err := s.reload(ctx); err != nil {
		return
	}
	if err := s.lock(ctx); err != nil {
		return
	}
	session := s.session
	if current := s.xstsTokens[relyingParty]; current != nil && current.Token == rejected.Token {
		delete(s.xstsTokens, relyingParty)
	}
	s.unlock()
	session.InvalidateXSTSToken(relyingParty, rejected)
	s.diagnostic("invalidate", "xsts", "rejected")
	s.publish(ctx)
}

// MultiplayerToken mints a key-bound multiplayer token from the shared service token.
func (s *Account) MultiplayerToken(ctx context.Context, key *ecdsa.PublicKey) (string, error) {
	if err := s.begin(); err != nil {
		return "", err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if key == nil {
		return "", errors.New("authentication: connection proof key is absent")
	}
	env, err := s.Environment(ctx)
	if err != nil {
		return "", err
	}
	jwt, err := s.deps.mint(ctx, env, s, key)
	if err != nil {
		if ctx.Err() != nil {
			return "", ctx.Err()
		}
		return "", errors.New("authentication: mint multiplayer credential")
	}
	return jwt, nil
}

// SessionID returns the Session-Id the account's service requests send; it is fixed for the account's life.
func (s *Account) SessionID() string { return s.sessionID }

// ServiceToken returns the account's Minecraft service token from the shared native source,
// persisting it so other processes reuse it.
func (s *Account) ServiceToken(ctx context.Context) (*service.Token, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if err := s.prepare(ctx); err != nil {
		return nil, err
	}
	return s.serviceToken(ctx)
}

// serviceToken returns the valid shared service token, or the one its flight exchanges.
func (s *Account) serviceToken(ctx context.Context) (*service.Token, error) {
	if _, err := s.ensureEnvironment(ctx); err != nil {
		return nil, err
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	token := s.service
	s.unlock()
	if token != nil && token.Valid() {
		s.diagnostic("reuse", "service", "valid")
		return token, nil
	}
	return awaitFlight(s, ctx, "service", s.exchangeService)
}

// exchangeService refreshes the shared service token; a failure keeps the current token.
func (s *Account) exchangeService(ctx context.Context) (*service.Token, error) {
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	env, source, seed, binding := s.environment, s.services, s.service, s.binding
	if source == nil && env != nil {
		source = s.deps.services(env, sessionTickets{s}, seed, s.serviceDeviceIDLocked(), s.sessionID)
	}
	s.unlock()
	if env == nil {
		return nil, errors.New("authentication: account changed during service refresh")
	}
	token, err := source.ServiceToken(ctx)
	if err != nil || token == nil || !token.Valid() {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, errors.New("authentication: refresh service credential")
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	install := s.binding == binding && s.environment == env && token != s.service
	if install {
		s.service, s.services = token, source
	}
	s.unlock()
	if install {
		s.diagnostic("refresh", "service", "expired")
		s.publish(ctx)
	}
	return token, nil
}

// InvalidateServiceToken drops a service token a service refused and persists the eviction.
func (s *Account) InvalidateServiceToken(rejected *service.Token) {
	if rejected == nil {
		return
	}
	if s.begin() != nil {
		return
	}
	defer s.end()
	ctx, cancel := context.WithTimeout(s.ctx, 10*time.Second)
	defer cancel()
	if err := s.reload(ctx); err != nil {
		return
	}
	if err := s.lock(ctx); err != nil {
		return
	}
	// The native source only ever caches s.service, so dropping both evicts the rejected token.
	if s.service != nil && s.service.AuthorizationHeader == rejected.AuthorizationHeader {
		s.service = nil
		s.services = nil
	}
	s.unlock()
	s.diagnostic("invalidate", "service", "rejected")
	s.publish(ctx)
}

// Environment returns the discovered authorization environment.
func (s *Account) Environment(ctx context.Context) (*service.AuthorizationEnvironment, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if err := s.prepare(ctx); err != nil {
		return nil, err
	}
	return s.ensureEnvironment(ctx)
}

// PlayFab returns the account's shared PlayFab client, logging in on first use; the account owns it.
func (s *Account) PlayFab(ctx context.Context) (*playfab.Client, error) {
	if err := s.begin(); err != nil {
		return nil, err
	}
	defer s.end()
	ctx, cancel := s.operationContext(ctx)
	defer cancel()
	if err := s.prepare(ctx); err != nil {
		return nil, err
	}
	return s.playFab(ctx)
}

// Closed reports whether shutdown, cancellation or a replaced sign-in ended this runtime.
func (s *Account) Closed() bool { return s.ctx.Err() != nil }

// Close cancels account operations, waits for those already running, ends the PlayFab session and
// refuses further calls.
func (s *Account) Close() error {
	s.cancel()
	s.drain()
	s.gate <- struct{}{}
	defer s.unlock()
	s.closed.Store(true)
	s.services = nil
	return s.closePlayFabLocked()
}

func (s *Account) ensureEnvironment(ctx context.Context) (*service.AuthorizationEnvironment, error) {
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	env := s.environment
	s.unlock()
	if env != nil {
		return env, nil
	}
	return awaitFlight(s, ctx, "environment", s.discoverEnvironment)
}

func (s *Account) discoverEnvironment(ctx context.Context) (*service.AuthorizationEnvironment, error) {
	env, err := s.deps.discover(ctx)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, errors.New("authentication: discover service environment")
	}
	if !validEnvironment(env) {
		return nil, errors.New("authentication: invalid service environment")
	}
	fresh := snapshotEnvironment(env)
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	defer s.unlock()
	if !sameEnvironment(s.cachedEnv, fresh) {
		s.service = nil
		s.services = nil
		s.diagnostic("miss", "service", "environment")
	}
	s.environment = env
	s.cachedEnv = fresh
	return env, nil
}

func (s *Account) playFab(ctx context.Context) (*playfab.Client, error) {
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	client := s.playfab
	s.unlock()
	if client != nil {
		return client, nil
	}
	env, err := s.ensureEnvironment(ctx)
	if err != nil {
		return nil, err
	}
	return awaitFlight(s, ctx, "playfab", func(ctx context.Context) (*playfab.Client, error) {
		return s.loginPlayFab(ctx, env)
	})
}

func (s *Account) loginPlayFab(ctx context.Context, env *service.AuthorizationEnvironment) (*playfab.Client, error) {
	client, err := s.deps.login(ctx, env, s.resolver)
	if err != nil {
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, errors.New("authentication: PlayFab login")
	}
	if err := s.lock(ctx); err != nil {
		_ = client.Close()
		return nil, err
	}
	defer s.unlock()
	if s.playfab != nil {
		_ = client.Close()
		return s.playfab, nil
	}
	// A source can detect account replacement outside the account gate during SISU refresh.
	context.AfterFunc(s.ctx, func() { _ = client.Close() })
	s.playfab = client
	return client, nil
}

func (s *Account) closePlayFabLocked() error {
	if s.playfab == nil {
		return nil
	}
	err := s.playfab.Close()
	s.playfab = nil
	return err
}

// sessionTickets hands the native service-token source the account's shared PlayFab session.
type sessionTickets struct{ account *Account }

func (t sessionTickets) SessionTicket(ctx context.Context) (string, error) {
	client, err := t.account.playFab(ctx)
	if err != nil {
		return "", err
	}
	return client.SessionTicket(ctx)
}

// lock serializes account state while allowing queued callers to cancel their wait.
func (s *Account) lock(ctx context.Context) error {
	if s.closed.Load() {
		return ErrAccountClosed
	}
	select {
	case s.gate <- struct{}{}:
		if s.closed.Load() {
			s.unlock()
			return ErrAccountClosed
		}
		return nil
	case <-ctx.Done():
		if s.closed.Load() {
			return ErrAccountClosed
		}
		return ctx.Err()
	}
}

// unlock lets the next queued account operation access the protected state.
func (s *Account) unlock() { <-s.gate }

// operationContext ends a caller's work when either it or the account closes.
func (s *Account) operationContext(ctx context.Context) (context.Context, context.CancelFunc) {
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(s.ctx, cancel)
	if s.ctx.Err() != nil {
		cancel()
	}
	return ctx, func() { stop(); cancel() }
}

// oauthToken propagates cancellation into cache leases. An already-running
// refresh may still finish and persist its rotation before cancellation is returned.
func (s *Account) oauthToken(ctx context.Context) (*oauth2.Token, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	var token *oauth2.Token
	var err error
	if cached, ok := s.oauth.(*persistingSource); ok {
		token, err = cached.token(ctx)
	} else {
		token, err = s.oauth.Token()
	}
	if errors.Is(err, errAccountChanged) {
		s.closed.Store(true)
		s.cancel()
		return nil, err
	}
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	return token, err
}

// currentOAuth returns the OAuth credential and applies its binding: adopt records a rotation the
// account's own SISU refresh made, otherwise new token material resets the derived state.
func (s *Account) currentOAuth(ctx context.Context, adopt bool) (*oauth2.Token, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	select {
	case s.oauthGate <- struct{}{}:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	defer func() { <-s.oauthGate }()
	token, err := s.oauthToken(ctx)
	if err != nil || token == nil {
		if errors.Is(err, errAccountChanged) {
			return nil, err
		}
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
		return nil, errors.New("authentication: validate OAuth credential")
	}
	if err := s.lock(ctx); err != nil {
		return nil, err
	}
	defer s.unlock()
	if binding := oauthBinding(token); binding != s.binding {
		if adopt {
			s.binding = binding
		} else {
			s.resetLocked(binding)
		}
	}
	return token, nil
}

// prepare applies the OAuth binding and adopts state another process published.
func (s *Account) prepare(ctx context.Context) error {
	if _, err := s.currentOAuth(ctx, false); err != nil {
		return err
	}
	return s.reload(ctx)
}

// reload adopts state another process published, holding the cache lease only for the local read.
func (s *Account) reload(ctx context.Context) error {
	lease, err := s.acquireLease(ctx)
	if err != nil || lease == nil {
		return err
	}
	defer lease.Close()
	if err := s.lock(ctx); err != nil {
		return err
	}
	defer s.unlock()
	s.reloadLocked()
	return nil
}

// acquireLease bounds waits for the optional derived cache. A miss keeps
// the account usable in memory without publishing over another process's state.
func (s *Account) acquireLease(ctx context.Context) (io.Closer, error) {
	if s.path == "" {
		return nil, nil
	}
	wait, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	lease, err := lockfile.AcquireContext(wait, s.path+cacheLockSuffix)
	if err == nil {
		return lease, nil
	}
	if ctx.Err() != nil {
		return nil, ctx.Err()
	}
	s.diagnostic("miss", "write", "unavailable")
	return nil, nil
}

// reloadLocked adopts a bundle another process published since this account last read or wrote it.
func (s *Account) reloadLocked() {
	if s.path == "" {
		return
	}
	state, fingerprint, err := loadDerivedBundle(s.path)
	if err != nil || fingerprint == s.persisted || state.OAuthBinding != s.binding || state.ClientBinding != s.client {
		return
	}
	if s.restore(state) == nil {
		s.persisted = fingerprint
		for relyingParty, token := range s.rejected {
			s.session.InvalidateXSTSToken(relyingParty, token)
			delete(s.xstsTokens, relyingParty)
		}
	}
}

// publish writes the account's derived state for other processes. It never replaces a bundle another
// process published since this account last read or wrote one; the next reload adopts that instead.
func (s *Account) publish(ctx context.Context) {
	if s.path == "" {
		return
	}
	if err := s.lock(ctx); err != nil {
		return
	}
	session := s.session
	s.unlock()
	// SISU holds its own locks across requests, so the snapshot is never taken under the gate.
	snapshot := session.Snapshot()
	lease, err := s.acquireLease(ctx)
	if err != nil || lease == nil {
		return
	}
	defer lease.Close()
	if err := s.lock(ctx); err != nil {
		return
	}
	defer s.unlock()
	if s.session != session {
		return
	}
	state, fingerprint, err := loadDerivedBundle(s.path)
	if err == nil && fingerprint != s.persisted && state.OAuthBinding == s.binding && state.ClientBinding == s.client {
		return
	}
	s.persistLocked(snapshot)
}

func (s *Account) persistLocked(snapshot *sisu.Snapshot) {
	device, proofKey := s.deviceToken, s.device.ProofKey()
	if device == nil || proofKey == nil {
		return
	}
	if snapshot != nil {
		// A snapshot taken before an invalidation finished may still hold the refused token.
		for relyingParty, token := range snapshot.XSTSTokens {
			if s.rejectedLocked(relyingParty, token) {
				delete(snapshot.XSTSTokens, relyingParty)
			}
		}
	}
	key, err := x509.MarshalECPrivateKey(proofKey)
	if err != nil {
		return
	}
	environment := snapshotEnvironment(s.environment)
	if environment == nil {
		environment = s.cachedEnv
	}
	state := derivedState{
		Version:       derivedCacheVersion,
		OAuthBinding:  s.binding,
		ClientBinding: s.client,
		Environment:   environment,
		DeviceToken:   device,
		ProofKey:      base64.RawStdEncoding.EncodeToString(key),
		SISU:          snapshot,
		ServiceToken:  s.service,
	}
	b, err := json.Marshal(state)
	if err != nil || len(b)+1 >= maxCacheSize {
		return
	}
	b = append(b, '\n')
	fingerprint := bytesFingerprint(b)
	if fingerprint == s.persisted {
		return
	}
	if err := savePrivate(s.path, b); err != nil {
		s.diagnostic("miss", "write", "contended")
		return
	}
	s.persisted = fingerprint
}

func (s *Account) resetLocked(binding string) {
	var proofKey *ecdsa.PrivateKey
	if s.device != nil {
		proofKey = s.device.ProofKey()
	}
	s.binding = binding
	s.environment = nil
	s.cachedEnv = nil
	s.service = nil
	s.services = nil // the PlayFab session is kept: a reset is an OAuth rotation of this process's account
	s.deviceToken = nil
	s.rejected = nil
	s.xstsTokens = make(map[string]*xsts.Token)
	s.device = xasd.ReuseTokenSource(auth.AndroidConfig.Config.Config, nil, proofKey)
	s.session = newAccountSession(s, &sisu.SessionConfig{DeviceTokenSource: s.device})
	s.persisted = ""
}

func (s *Account) restore(state *derivedState) error {
	if state == nil || state.DeviceToken == nil || state.ProofKey == "" || state.SISU == nil {
		return errDerivedCacheMiss
	}
	der, err := base64.RawStdEncoding.DecodeString(state.ProofKey)
	if err != nil {
		return errDerivedCacheMiss
	}
	key, err := x509.ParseECPrivateKey(der)
	if err != nil || key.Curve == nil || key.Curve.Params().Name != "P-256" {
		return errDerivedCacheMiss
	}
	if s.device != nil && s.device.ProofKey() != nil {
		current := s.device.ProofKey()
		if current.Curve == nil || current.Curve.Params().Name != key.Curve.Params().Name || current.D.Cmp(key.D) != 0 {
			return errDerivedCacheMiss
		}
		key = current
	}
	var cachedEnv *derivedEnvironment
	if state.Environment != nil {
		if _, err := restoreEnvironment(state.Environment); err != nil {
			return errDerivedCacheMiss
		}
		cachedEnv = state.Environment
	}
	device := xasd.ReuseTokenSource(auth.AndroidConfig.Config.Config, state.DeviceToken, key)
	tokens := maps.Clone(state.SISU.XSTSTokens) // SISU keeps and mutates the snapshot's own map
	if tokens == nil {
		tokens = make(map[string]*xsts.Token)
	}
	session := newAccountSession(s, &sisu.SessionConfig{Snapshot: state.SISU, DeviceTokenSource: device})
	var serviceToken *service.Token
	if state.ServiceToken != nil && state.ServiceToken.Valid() && cachedEnv != nil {
		serviceToken = state.ServiceToken
	}
	s.device = device
	s.deviceToken = state.DeviceToken
	s.session = session
	s.xstsTokens = tokens
	if !sameEnvironment(snapshotEnvironment(s.environment), cachedEnv) {
		s.environment = nil // a restored environment is checked against discovery once more
	}
	s.cachedEnv = cachedEnv
	// The in-memory copy of an unchanged token keeps the service clock its validity is judged by.
	if s.service == nil || cachedEnv == nil || state.ServiceToken == nil ||
		s.service.AuthorizationHeader != state.ServiceToken.AuthorizationHeader {
		s.service = serviceToken
	}
	s.services = nil
	return nil
}

var errDerivedCacheMiss = errors.New("derived authentication cache miss")

func loadDerived(path string) (*derivedState, error) {
	state, _, err := loadDerivedBundle(path)
	return state, err
}

// loadDerivedBundle also returns the fingerprint of the exact bytes read.
func loadDerivedBundle(path string) (*derivedState, string, error) {
	b, err := loadPrivate(path, maxCacheSize)
	if err != nil {
		return nil, "", err
	}
	decoder := json.NewDecoder(bytes.NewReader(b))
	decoder.DisallowUnknownFields()
	var state derivedState
	if err := decoder.Decode(&state); err != nil {
		return nil, "", errDerivedCacheMiss
	}
	var trailing any
	if err := decoder.Decode(&trailing); !errors.Is(err, io.EOF) {
		return nil, "", errDerivedCacheMiss
	}
	if state.Version != derivedCacheVersion || state.OAuthBinding == "" || state.ClientBinding == "" {
		return nil, "", errDerivedCacheMiss
	}
	return &state, bytesFingerprint(b), nil
}

func oauthBinding(token *oauth2.Token) string {
	h := sha256.New()
	for _, value := range []string{token.AccessToken, token.RefreshToken, token.TokenType} {
		var length [8]byte
		binary.BigEndian.PutUint64(length[:], uint64(len(value)))
		_, _ = h.Write(length[:])
		_, _ = h.Write([]byte(value))
	}
	return hex.EncodeToString(h.Sum(nil))
}

func bytesFingerprint(b []byte) string {
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

func clientBinding() string {
	b, _ := json.Marshal(struct {
		Config   auth.Config `json:"xal"`
		Protocol string      `json:"protocol"`
		App      string      `json:"application"`
	}{auth.AndroidConfig, protocol.CurrentVersion, service.ApplicationTypeMinecraftPE})
	sum := sha256.Sum256(b)
	return hex.EncodeToString(sum[:])
}

func snapshotEnvironment(env *service.AuthorizationEnvironment) *derivedEnvironment {
	if !validEnvironment(env) {
		return nil
	}
	return &derivedEnvironment{
		ServiceURI:     env.ServiceURI.String(),
		Issuer:         env.Issuer.String(),
		PlayFabTitleID: env.PlayFabTitleID,
	}
}

func restoreEnvironment(cached *derivedEnvironment) (*service.AuthorizationEnvironment, error) {
	serviceURI, err := url.Parse(cached.ServiceURI)
	if err != nil {
		return nil, err
	}
	issuer, err := url.Parse(cached.Issuer)
	if err != nil {
		return nil, err
	}
	env := &service.AuthorizationEnvironment{ServiceURI: serviceURI, Issuer: issuer, PlayFabTitleID: cached.PlayFabTitleID}
	if !validEnvironment(env) {
		return nil, fmt.Errorf("invalid environment")
	}
	return env, nil
}

func validEnvironment(env *service.AuthorizationEnvironment) bool {
	return env != nil && validHTTPSURL(env.ServiceURI) && validHTTPSURL(env.Issuer) && env.PlayFabTitleID != ""
}

func sameEnvironment(left, right *derivedEnvironment) bool {
	return left != nil && right != nil && left.ServiceURI == right.ServiceURI && left.Issuer == right.Issuer && left.PlayFabTitleID == right.PlayFabTitleID
}

func validHTTPSURL(value *url.URL) bool {
	return value != nil && value.Scheme == "https" && value.Host != "" && value.User == nil
}
