// Package launcher implements the control-channel launcher services: catalog
// listings, upstream selection, and sign-out.
package launcher

import (
	"context"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"time"

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
	ArtworkDir  string // screen artwork cache; empty skips caching
	CacheFile   string // last good catalog; empty keeps it in memory only
	Logger      *slog.Logger
	// StoreImageDir holds cached Marketplace images; empty disables them.
	StoreImageDir string

	// Injectable for tests; nil selects the real implementation.
	Realms   func(context.Context, oauth2.TokenSource) ([]catalog.Realm, error)
	Friends  func(context.Context, oauth2.TokenSource) ([]catalog.Friend, error)
	Gamertag func(context.Context, oauth2.TokenSource) (string, error)
	Remove   func(path string) error

	Featured   func(context.Context, oauth2.TokenSource) ([]catalog.FeaturedServer, error)
	Gatherings func(context.Context, oauth2.TokenSource) ([]catalog.Gathering, error)
	Profile    func(context.Context, oauth2.TokenSource) (catalog.Profile, error)
	CacheArt   func(ctx context.Context, directory string, images []*catalog.Image)
	Ping       func(ctx context.Context, addresses []string) []catalog.PingResult
	Home       func(ctx context.Context, src oauth2.TokenSource, session *catalog.MessagingSession, artworkDir string) (catalog.Home, error)
	Report     func(ctx context.Context, src oauth2.TokenSource, session *catalog.MessagingSession, event catalog.MessageEvent) error
}

// Service implements control.Services.
type Service struct {
	cfg       Config
	logger    *slog.Logger
	signedOut atomic.Bool
	messaging catalog.MessagingSession

	mu        sync.Mutex
	snap      snapshot
	flights   [3]*flight
	attempted [3]time.Time
	gamerpic  string     // profile artwork pruning must keep
	disk      sync.Mutex // orders cache rewrites
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
	if cfg.Featured == nil {
		cfg.Featured = catalog.FeaturedServers
	}
	if cfg.Gatherings == nil {
		cfg.Gatherings = catalog.Gatherings
	}
	if cfg.Profile == nil {
		cfg.Profile = catalog.AccountProfile
	}
	if cfg.CacheArt == nil {
		cfg.CacheArt = catalog.CacheImages
	}
	if cfg.Ping == nil {
		cfg.Ping = catalog.PingServers
	}
	if cfg.Home == nil {
		cfg.Home = catalog.HomeFeed
	}
	if cfg.Report == nil {
		cfg.Report = catalog.ReportMessageEvent
	}
	s := &Service{cfg: cfg, logger: cfg.Logger}
	if s.logger == nil {
		s.logger = slog.New(slog.DiscardHandler)
	}
	s.load()
	return s
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

// FeaturedServers lists the featured servers with their artwork cached, from the last good fetch.
func (s *Service) FeaturedServers(ctx context.Context) ([]catalog.FeaturedServer, error) {
	return cached(ctx, s, featuredFeed)
}

// Gatherings lists the community gatherings with their artwork cached, from the last good fetch.
func (s *Service) Gatherings(ctx context.Context) ([]catalog.Gathering, error) {
	return cached(ctx, s, gatheringsFeed)
}

// Profile returns the signed-in profile with its gamerpic cached.
func (s *Service) Profile(ctx context.Context) (catalog.Profile, error) {
	src, err := s.source()
	if err != nil {
		return catalog.Profile{}, err
	}
	profile, err := s.cfg.Profile(ctx, src)
	if err != nil {
		return catalog.Profile{}, err
	}
	s.cacheArt(ctx, []*catalog.Image{&profile.Gamerpic})
	s.mu.Lock()
	s.gamerpic = profile.Gamerpic.Path
	s.mu.Unlock()
	return profile, nil
}

// Home returns the start screen's service data with its artwork cached, from the last good fetch.
func (s *Service) Home(ctx context.Context) (catalog.Home, error) {
	return cached(ctx, s, homeFeed)
}

// ReportMessage posts one messaging report for the signed-in session.
func (s *Service) ReportMessage(ctx context.Context, event catalog.MessageEvent) error {
	src, err := s.source()
	if err != nil {
		return err
	}
	return s.cfg.Report(ctx, src, &s.messaging, event)
}

// Ping pings servers for their player counts and round trip; it needs no account.
func (s *Service) Ping(ctx context.Context, addresses []string) []catalog.PingResult {
	return s.cfg.Ping(ctx, addresses)
}

func (s *Service) cacheArt(ctx context.Context, images []*catalog.Image) {
	if s.cfg.ArtworkDir != "" {
		s.cfg.CacheArt(ctx, s.cfg.ArtworkDir, images)
	}
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
	s.mu.Lock()
	s.signedOut.Store(true)
	s.snap = snapshot{}
	s.mu.Unlock()
	var paths []string
	if s.cfg.AuthCache != "" {
		paths = append(paths, s.cfg.AuthCache, authcache.DerivedCachePath(s.cfg.AuthCache))
	}
	if s.cfg.CacheFile != "" {
		paths = append(paths, s.cfg.CacheFile)
	}
	var failed bool
	s.disk.Lock()
	for _, path := range paths {
		if err := s.cfg.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
			failed = true
		}
	}
	s.disk.Unlock()
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
