package authcache

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"path/filepath"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/df-mc/go-playfab/v2"
	"github.com/df-mc/go-xsapi/v2"
	"github.com/df-mc/go-xsapi/v2/xal"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"golang.org/x/oauth2"
)

// hangingTransport never connects, like a host whose TCP handshake never completes.
type hangingTransport struct{ started chan struct{} }

func (h hangingTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	select {
	case h.started <- struct{}{}:
	default:
	}
	<-req.Context().Done()
	return nil, req.Context().Err()
}

// hungLoginAccount restores a cached account whose expired service token needs a PlayFab login that hangs.
func hungLoginAccount(t *testing.T) (*Account, <-chan struct{}) {
	t.Helper()
	path := filepath.Join(derivedTestDir(t), "derived")
	oauthToken := testOAuthToken("account-a")
	writeDerivedState(t, path, oauthToken, time.Now().Add(-time.Minute))
	started := make(chan struct{}, 1)
	deps := defaultDerivedDeps()
	deps.discover = func(context.Context) (*service.AuthorizationEnvironment, error) { return testEnvironment(), nil }
	deps.login = func(ctx context.Context, _ *service.AuthorizationEnvironment, _ xsapi.TokenAndSignaturer) (*playfab.Client, error) {
		if _, ok := ctx.Deadline(); !ok || xal.ContextClient(ctx).Timeout == 0 {
			t.Error("PlayFab login ran without a deadline and a timed HTTP client")
		}
		req, err := http.NewRequestWithContext(ctx, http.MethodGet, "https://title.example.test/titles/current/endpoints", nil)
		if err != nil {
			return nil, err
		}
		_, err = (&http.Client{Transport: hangingTransport{started}}).Do(req)
		return nil, err
	}
	account := newAccount(context.Background(), path, oauth2.StaticTokenSource(oauthToken), nil, deps)
	t.Cleanup(func() { _ = account.Close() })
	return account, started
}

// A service-token refresh stuck on an unreachable host must not block a join's XSTS token.
func TestHungServiceRefreshDoesNotBlockOtherCredentials(t *testing.T) {
	account, started := hungLoginAccount(t)
	go func() { _, _ = account.ServiceToken(context.Background()) }()
	select {
	case <-started:
	case <-time.After(5 * time.Second):
		t.Fatal("service refresh never reached PlayFab login")
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if token, err := account.XSTSToken(ctx, cachedRelyingParty); err != nil || token.Token != "xsts-token" {
		t.Fatalf("XSTS during a hung refresh: token=%v err=%v", token, err)
	}
	if _, err := account.DeviceToken(ctx); err != nil {
		t.Fatalf("device token during a hung refresh: %v", err)
	}
	if _, err := account.Environment(ctx); err != nil {
		t.Fatalf("environment during a hung refresh: %v", err)
	}
	if account.ProofKey() == nil {
		t.Fatal("proof key unavailable during a hung refresh")
	}
}

// A caller leaving a hung refresh returns at its own deadline without cancelling other waiters.
func TestHungRefreshRespectsEachCallerContext(t *testing.T) {
	account, started := hungLoginAccount(t)
	patient, cancelPatient := context.WithCancel(context.Background())
	defer cancelPatient()
	waiting := make(chan error, 1)
	go func() { _, err := account.ServiceToken(patient); waiting <- err }()
	<-started

	short, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	began := time.Now()
	if _, err := account.ServiceToken(short); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("short caller = %v, want its own deadline", err)
	}
	if elapsed := time.Since(began); elapsed > time.Second {
		t.Fatalf("short caller returned after %v", elapsed)
	}
	select {
	case err := <-waiting:
		t.Fatalf("another caller's deadline ended this wait: %v", err)
	default:
	}
	cancelPatient()
	select {
	case err := <-waiting:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("patient caller = %v, want its own cancellation", err)
		}
	case <-time.After(time.Second):
		t.Fatal("patient caller ignored cancellation")
	}
	closed := make(chan error, 1)
	go func() { closed <- account.Close() }()
	select {
	case <-closed:
	case <-time.After(5 * time.Second):
		t.Fatal("Close waited on the hung refresh")
	}
}

// Concurrent first uses of the PlayFab session share one login.
func TestConcurrentPlayFabCallersShareOneLogin(t *testing.T) {
	var logins atomic.Int32
	release := make(chan struct{})
	deps := derivedDeps{
		discover: func(context.Context) (*service.AuthorizationEnvironment, error) { return testEnvironment(), nil },
		login: func(ctx context.Context, env *service.AuthorizationEnvironment, _ xsapi.TokenAndSignaturer) (*playfab.Client, error) {
			<-release
			return playfab.Login(ctx, env.PlayFabTitleID, fakeIdentityProvider{&logins}, playfab.ClientConfig{
				HTTPClient: &http.Client{Transport: refusingTransport{}}, Logger: slog.New(slog.DiscardHandler),
			})
		},
	}
	account := newAccount(context.Background(), "", oauth2.StaticTokenSource(testOAuthToken("account-a")), nil, deps)
	defer account.Close()
	clients := make(chan *playfab.Client, 4)
	var wg sync.WaitGroup
	for range cap(clients) {
		wg.Go(func() {
			client, err := account.PlayFab(context.Background())
			if err != nil {
				t.Error(err)
			}
			clients <- client
		})
	}
	time.Sleep(50 * time.Millisecond)
	close(release)
	wg.Wait()
	close(clients)
	first := <-clients
	for client := range clients {
		if client != first {
			t.Fatal("concurrent callers received different PlayFab sessions")
		}
	}
	if logins.Load() != 1 {
		t.Fatalf("logins = %d, want 1", logins.Load())
	}
}
