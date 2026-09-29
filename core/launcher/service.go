// Package launcher implements the control-channel launcher services: catalog
// listings, upstream selection, and sign-out.
package launcher

import (
	"context"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"strconv"
	"strings"
	"sync/atomic"

	"github.com/hashimthearab/rust-mcbe/core/authcache"
	"github.com/hashimthearab/rust-mcbe/core/catalog"
	"github.com/hashimthearab/rust-mcbe/core/control"
	"github.com/hashimthearab/rust-mcbe/core/proxy"
	"github.com/sandertv/gophertunnel/minecraft/auth"
	"golang.org/x/oauth2"
)

// Config wires a Service. A nil TokenSource means the core runs without an account.
type Config struct {
	TokenSource oauth2.TokenSource
	AuthCache   string // token cache path; sign-out deletes it and its derived cache
	Store       *control.Store
	Selector    *proxy.UpstreamSelector
	Transfers   *proxy.TransferState

	// Injectable for tests; nil selects the real implementation.
	Realms   func(context.Context, oauth2.TokenSource) ([]catalog.Realm, error)
	Friends  func(context.Context, oauth2.TokenSource) ([]catalog.Friend, error)
	Gamertag func(context.Context, oauth2.TokenSource) (string, error)
	Remove   func(path string) error
}

// Service implements control.Services.
type Service struct {
	cfg       Config
	signedOut atomic.Bool
}

// New returns a Service; it fills unset injectables with the real implementations.
func New(cfg Config) *Service {
	if cfg.Realms == nil {
		cfg.Realms = catalog.Realms
	}
	if cfg.Friends == nil {
		cfg.Friends = catalog.Friends
	}
	if cfg.Gamertag == nil {
		cfg.Gamertag = catalog.Gamertag
	}
	if cfg.Remove == nil {
		cfg.Remove = os.Remove
	}
	return &Service{cfg: cfg}
}

func (s *Service) source() (oauth2.TokenSource, error) {
	if s.cfg.TokenSource == nil || s.signedOut.Load() {
		return nil, control.ErrSignedOut
	}
	return s.cfg.TokenSource, nil
}

// Realms lists the account's Realms.
func (s *Service) Realms(ctx context.Context) ([]catalog.Realm, error) {
	src, err := s.source()
	if err != nil {
		return nil, err
	}
	return s.cfg.Realms(ctx, src)
}

// Friends lists joinable friend worlds.
func (s *Service) Friends(ctx context.Context) ([]catalog.Friend, error) {
	src, err := s.source()
	if err != nil {
		return nil, err
	}
	return s.cfg.Friends(ctx, src)
}

// Connect selects the upstream for the next client connection and drops any pending transfer.
func (s *Service) Connect(kind, value string) error {
	target, err := upstreamTarget(kind, value)
	if err != nil {
		return err
	}
	if kind != control.TargetRakNet {
		if _, err := s.source(); err != nil {
			return err
		}
	}
	if s.cfg.Selector != nil {
		s.cfg.Selector.Set(target)
	}
	if s.cfg.Transfers != nil {
		s.cfg.Transfers.Clear()
	}
	if s.cfg.Store != nil {
		s.cfg.Store.ClearTransfer()
	}
	return nil
}

// upstreamTarget maps a connect.v1 target to the proxy's target syntax.
func upstreamTarget(kind, value string) (string, error) {
	value = strings.TrimSpace(value)
	switch kind {
	case control.TargetRakNet:
		host, port, err := net.SplitHostPort(value)
		number, portErr := strconv.ParseUint(port, 10, 16)
		if err != nil || portErr != nil || number == 0 || host == "" || strings.ContainsAny(host, " \t/\\") {
			return "", control.ErrInvalidTarget
		}
		return net.JoinHostPort(host, port), nil
	case control.TargetRealm:
		if id, err := strconv.ParseUint(value, 10, 31); err != nil || id == 0 {
			return "", control.ErrInvalidTarget
		}
		return "realm_id/" + value, nil
	case control.TargetFriend:
		if _, err := strconv.ParseUint(value, 10, 64); err != nil {
			return "", control.ErrInvalidTarget
		}
		return "friend_xuid/" + value, nil
	}
	return "", control.ErrInvalidTarget
}

// SignOut deletes the cached Microsoft tokens and reports the signed-out state. The running
// process stops using the account; a new sign-in needs the device-code flow and a core restart.
func (s *Service) SignOut() error {
	if s.cfg.TokenSource == nil {
		return control.ErrSignedOut
	}
	s.signedOut.Store(true)
	var failed bool
	if s.cfg.AuthCache != "" {
		for _, path := range []string{s.cfg.AuthCache, authcache.DerivedCachePath(s.cfg.AuthCache)} {
			if err := s.cfg.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
				failed = true
			}
		}
	}
	if s.cfg.Selector != nil {
		s.cfg.Selector.Set("")
	}
	if s.cfg.Store != nil {
		s.cfg.Store.SetAuth(control.AuthV1{State: control.AuthSignedOut})
	}
	if failed {
		return errors.New("launcher: remove cached tokens")
	}
	return nil
}

// PublishSignedIn reports the signed-in state with the gamertag when it can be read.
func (s *Service) PublishSignedIn(ctx context.Context) {
	if s.cfg.Store == nil || s.cfg.TokenSource == nil {
		return
	}
	state := control.AuthV1{State: control.AuthSignedIn}
	if tag, err := s.cfg.Gamertag(ctx, s.cfg.TokenSource); err == nil {
		state.Gamertag = tag
	}
	if s.signedOut.Load() {
		return
	}
	s.cfg.Store.SetAuth(state)
}

// DeviceRequest is an authcache.Config.Request that publishes the device code to store and
// writes the standard prompt line to w; it publishes a sanitized failure reason on error.
func DeviceRequest(store *control.Store) func(context.Context, io.Writer) (*oauth2.Token, error) {
	return func(ctx context.Context, w io.Writer) (*oauth2.Token, error) {
		fail := func(reason string, err error) (*oauth2.Token, error) {
			if store != nil {
				store.SetAuth(control.AuthV1{State: control.AuthFailed, Reason: reason})
			}
			return nil, err
		}
		device, err := auth.AndroidConfig.DeviceAuth(ctx)
		if err != nil {
			return fail("Could not start Microsoft sign-in.", fmt.Errorf("start device auth: %w", err))
		}
		if store != nil {
			store.SetAuth(control.AuthV1{
				State: control.AuthAwaitingCode, VerificationURI: device.VerificationURI, UserCode: device.UserCode,
			})
		}
		_, _ = fmt.Fprintf(w, "Authenticate at %v using the code %v.\n", device.VerificationURI, device.UserCode)
		token, err := auth.AndroidConfig.DeviceAccessToken(ctx, device)
		if err != nil {
			return fail("Microsoft sign-in did not complete.", fmt.Errorf("poll device token: %w", err))
		}
		_, _ = w.Write([]byte("Authentication successful.\n"))
		return token, nil
	}
}
