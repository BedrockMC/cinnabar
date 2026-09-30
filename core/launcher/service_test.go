package launcher

import (
	"context"
	"errors"
	"os"
	"strings"
	"testing"

	"github.com/hashimthearab/rust-mcbe/core/catalog"
	"github.com/hashimthearab/rust-mcbe/core/control"
	"github.com/hashimthearab/rust-mcbe/core/proxy"
	"golang.org/x/oauth2"
)

type staticSource struct{}

func (staticSource) Token() (*oauth2.Token, error) { return &oauth2.Token{AccessToken: "x"}, nil }

type fixture struct {
	service  *Service
	store    *control.Store
	selector *proxy.UpstreamSelector
	removed  []string
}

func newFixture(t *testing.T, source oauth2.TokenSource) *fixture {
	t.Helper()
	f := &fixture{store: control.NewStore(), selector: new(proxy.UpstreamSelector)}
	f.service = New(Config{
		TokenSource: source, AuthCache: "/cache/token.json",
		Store: f.store, Selector: f.selector, Transfers: new(proxy.TransferState),
		Realms: func(context.Context, oauth2.TokenSource) ([]catalog.Realm, error) {
			return []catalog.Realm{{Name: "R", Target: "realm_id/1"}}, nil
		},
		Friends: func(context.Context, oauth2.TokenSource) ([]catalog.Friend, error) {
			return []catalog.Friend{{XUID: "9"}}, nil
		},
		Gamertag: func(context.Context, oauth2.TokenSource) (string, error) { return "Steve", nil },
		Remove:   func(path string) error { f.removed = append(f.removed, path); return nil },
	})
	return f
}

func TestConnectMapsTargetsToProxySyntax(t *testing.T) {
	f := newFixture(t, staticSource{})
	for _, test := range []struct{ kind, value, want string }{
		{control.TargetRakNet, " play.example.net:19132 ", "play.example.net:19132"},
		{control.TargetRakNet, "[::1]:19132", "[::1]:19132"},
		{control.TargetRealm, "12345", "realm_id/12345"},
		{control.TargetFriend, "2535428000000000", "friend_xuid/2535428000000000"},
	} {
		if err := f.service.Connect(test.kind, test.value); err != nil {
			t.Fatalf("Connect(%s, %q) = %v", test.kind, test.value, err)
		}
		if got, _ := f.selector.Target(); got != test.want {
			t.Fatalf("selected %q, want %q", got, test.want)
		}
	}
}

func TestConnectRejectsMalformedTargetsWithoutChangingSelection(t *testing.T) {
	f := newFixture(t, staticSource{})
	_ = f.service.Connect(control.TargetRakNet, "keep.example:1")
	for _, test := range []struct{ kind, value string }{
		{control.TargetRakNet, "no-port"}, {control.TargetRakNet, "host:0"}, {control.TargetRakNet, "host:99999"},
		{control.TargetRakNet, "a b:1"}, {control.TargetRakNet, ":19132"}, {control.TargetRealm, "0"},
		{control.TargetRealm, "abc"}, {control.TargetFriend, "gamertag"}, {"other", "x"},
	} {
		if err := f.service.Connect(test.kind, test.value); !errors.Is(err, control.ErrInvalidTarget) {
			t.Fatalf("Connect(%s, %q) = %v, want invalid target", test.kind, test.value, err)
		}
	}
	if got, _ := f.selector.Target(); got != "keep.example:1" {
		t.Fatalf("selection changed to %q", got)
	}
}

func TestConnectClearsPendingTransfer(t *testing.T) {
	f := newFixture(t, staticSource{})
	f.store.ObserveTransfer(proxy.TransferTarget{Host: "next", Port: 1})
	if err := f.service.Connect(control.TargetRakNet, "a.example:1"); err != nil {
		t.Fatal(err)
	}
	if f.store.Status().Transfer != nil {
		t.Fatal("explicit connect left the transfer pending")
	}
}

func TestAccountBoundOperationsNeedASession(t *testing.T) {
	f := newFixture(t, nil)
	if _, err := f.service.Realms(context.Background()); !errors.Is(err, control.ErrSignedOut) {
		t.Fatalf("Realms() = %v", err)
	}
	if _, err := f.service.Friends(context.Background()); !errors.Is(err, control.ErrSignedOut) {
		t.Fatalf("Friends() = %v", err)
	}
	if err := f.service.Connect(control.TargetRealm, "5"); !errors.Is(err, control.ErrSignedOut) {
		t.Fatalf("realm Connect() = %v", err)
	}
	if err := f.service.Connect(control.TargetRakNet, "a.example:1"); err != nil {
		t.Fatalf("raknet Connect() without account = %v", err)
	}
}

func TestSignOutRemovesCachesAndBlocksAccountCalls(t *testing.T) {
	f := newFixture(t, staticSource{})
	f.store.SetAuth(control.AuthV1{State: control.AuthSignedIn, Gamertag: "Steve"})
	_ = f.service.Connect(control.TargetRealm, "5")
	if err := f.service.SignOut(); err != nil {
		t.Fatal(err)
	}
	if len(f.removed) != 2 || f.removed[0] != "/cache/token.json" || f.removed[1] == f.removed[0] {
		t.Fatalf("removed %v, want the token cache and its derived cache", f.removed)
	}
	if got := f.store.Auth(); got.State != control.AuthSignedOut || got.Gamertag != "" {
		t.Fatalf("auth = %+v", got)
	}
	if _, ok := f.selector.Target(); ok {
		t.Fatal("sign-out kept the account-bound selection")
	}
	if _, err := f.service.Realms(context.Background()); !errors.Is(err, control.ErrSignedOut) {
		t.Fatalf("Realms() after sign-out = %v", err)
	}
	f.service.PublishSignedIn(context.Background())
	if f.store.Auth().State != control.AuthSignedOut {
		t.Fatal("late sign-in publication overwrote the signed-out state")
	}
}

func TestSignOutToleratesMissingFilesAndReportsRealFailures(t *testing.T) {
	f := newFixture(t, staticSource{})
	f.service.cfg.Remove = func(string) error { return os.ErrNotExist }
	if err := f.service.SignOut(); err != nil {
		t.Fatalf("missing cache files: %v", err)
	}
	f = newFixture(t, staticSource{})
	f.service.cfg.Remove = func(string) error { return errors.New("/secret/path: permission denied") }
	if err := f.service.SignOut(); err == nil || strings.Contains(err.Error(), "/secret") {
		t.Fatalf("SignOut() = %v, want a failure", err)
	}
	if f.store.Auth().State != control.AuthSignedOut {
		t.Fatal("failed removal must still report signed out")
	}
}

func TestPublishSignedInIncludesGamertag(t *testing.T) {
	f := newFixture(t, staticSource{})
	f.service.PublishSignedIn(context.Background())
	if got := f.store.Auth(); got.State != control.AuthSignedIn || got.Gamertag != "Steve" {
		t.Fatalf("auth = %+v", got)
	}
}

func TestScreenFeedsCacheArtworkAndNeedAnAccount(t *testing.T) {
	var cached []string
	service := New(Config{
		TokenSource: staticSource{}, ArtworkDir: "/art",
		Featured: func(context.Context, oauth2.TokenSource) ([]catalog.FeaturedServer, error) {
			return []catalog.FeaturedServer{{Name: "S", Logo: catalog.Image{URL: "https://a.test/l.png"}}}, nil
		},
		Profile: func(context.Context, oauth2.TokenSource) (catalog.Profile, error) {
			return catalog.Profile{Gamertag: "Steve"}, nil
		},
		CacheArt: func(_ context.Context, directory string, images []*catalog.Image) {
			for _, image := range images {
				if image.URL != "" {
					image.Path = directory + "/cached"
					cached = append(cached, image.URL)
				}
			}
		},
	})
	servers, err := service.FeaturedServers(context.Background())
	if err != nil || len(servers) != 1 || servers[0].Logo.Path != "/art/cached" || len(cached) != 1 {
		t.Fatalf("servers = %+v, err = %v, cached = %v", servers, err, cached)
	}
	if profile, err := service.Profile(context.Background()); err != nil || profile.Gamertag != "Steve" {
		t.Fatalf("profile = %+v, err = %v", profile, err)
	}
	offline := New(Config{})
	if _, err := offline.Gatherings(context.Background()); !errors.Is(err, control.ErrSignedOut) {
		t.Fatalf("offline gatherings err = %v", err)
	}
}

func TestHomeCachesMessageAndEventArtwork(t *testing.T) {
	service := New(Config{
		TokenSource: staticSource{}, ArtworkDir: "/art",
		Home: func(context.Context, oauth2.TokenSource, *catalog.MessagingSession, string) (catalog.Home, error) {
			return catalog.Home{
				Messages: []catalog.Message{{ID: "m", Images: []catalog.MessageImage{
					{ID: "tile", Image: catalog.Image{URL: "https://a.test/t.png"}},
				}}},
				LiveEvents: []catalog.LiveEvent{{ID: "g", Badge: catalog.Image{URL: "https://a.test/b.png"}}},
			}, nil
		},
		CacheArt: func(_ context.Context, directory string, images []*catalog.Image) {
			for _, image := range images {
				if image.URL != "" {
					image.Path = directory + "/cached"
				}
			}
		},
	})
	home, err := service.Home(context.Background())
	if err != nil || home.Messages[0].Images[0].Path != "/art/cached" || home.LiveEvents[0].Badge.Path != "/art/cached" {
		t.Fatalf("home = %+v, err = %v", home, err)
	}
}
