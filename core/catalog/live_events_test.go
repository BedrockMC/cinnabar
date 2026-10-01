package catalog

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/hashimthearab/rust-mcbe/core/clientplatform"
	"github.com/sandertv/gophertunnel/minecraft/protocol"
	"github.com/sandertv/gophertunnel/minecraft/service"
)

// This is authored offline data, not a captured service response.
const liveEventResponse = `{"result":[{"gatheringId":"promo","title":"Live event",
"endTimeUtc":"2026-12-01T00:00:00Z","shouldRouteToServerTab":true,
"segments":[{"ui":{"startScreenButtonText":"Learn More","captionText":"Live now",
"badgeImage":"https://cdn.example.test/promo.png"}}]}]}`

func TestDesktopPublicConfigReachesHomeArtwork(t *testing.T) {
	home := replayPublicConfig(t, liveEventResponse, time.Date(2026, 10, 1, 0, 0, 0, 0, time.UTC))
	if home.LiveEvents[0].ButtonText != "Learn More" || home.LiveEvents[0].CaptionText != "Live now" || !home.LiveEvents[0].RouteToServers {
		t.Fatalf("promo = %+v", home.LiveEvents[0])
	}
}

func TestRecordedPublicConfigReachesHomeArtwork(t *testing.T) {
	path := os.Getenv("CINNABAR_GATHERING_RESPONSE_FIXTURE")
	if path == "" {
		t.Skip("no recorded public-config response supplied; synthetic coverage is separate")
	}
	response, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	now, err := time.Parse(time.RFC3339, os.Getenv("CINNABAR_GATHERING_FIXTURE_TIME"))
	if err != nil {
		t.Fatalf("supply the recording time in CINNABAR_GATHERING_FIXTURE_TIME: %v", err)
	}
	replayPublicConfig(t, string(response), now)
}

// replayPublicConfig exercises the offline request, event mapping and cached artwork handoff.
func replayPublicConfig(t *testing.T, response string, now time.Time) Home {
	t.Helper()
	previous := http.DefaultClient.Transport
	http.DefaultClient.Transport = roundTripFunc(func(req *http.Request) (*http.Response, error) {
		query := req.URL.Query()
		if req.Method != http.MethodGet || req.URL.Host != "gatherings.discovered.example" || req.URL.Path != "/api/v1.0/config/public" ||
			query.Get("clientVersion") != protocol.CurrentVersion ||
			query.Get("clientPlatform") != clientplatform.Platform || query.Get("clientSubPlatform") != clientplatform.SubPlatform {
			t.Fatalf("desktop config request = %s %s", req.Method, req.URL)
		}
		if req.Header.Get("Authorization") != "MCToken synthetic" {
			t.Fatal("config request did not use the supplied service token")
		}
		return &http.Response{StatusCode: http.StatusOK, Body: io.NopCloser(strings.NewReader(response)), Header: make(http.Header)}, nil
	})
	t.Cleanup(func() { http.DefaultClient.Transport = previous })
	discovery := &service.Discovery{ServiceEnvironments: map[string]map[string]json.RawMessage{
		"gatherings": {"prod": json.RawMessage(`{"serviceUri":"https://gatherings.discovered.example"}`)},
	}}
	events, err := liveEvents(context.Background(), discovery, fixedTokens{}, now)
	if err != nil || len(events) == 0 {
		t.Fatalf("events = %+v, err = %v", events, err)
	}
	home := Home{
		Messages: []Message{}, Inbox: Inbox{Categories: []InboxCategory{}},
		Treatments: []string{}, LiveEvents: events,
	}
	images := HomeImages(&home)
	if len(images) == 0 || home.LiveEvents[0].Badge.URL == "" {
		t.Fatal("fixture must contain an active event with badge artwork")
	}
	home.LiveEvents[0].Badge.Path = "/offline/promo.png"
	if images[0].Path != home.LiveEvents[0].Badge.Path {
		t.Fatal("cached badge path did not reach the home feed")
	}
	if path := os.Getenv("CINNABAR_HOME_PROMO_FIXTURE"); path != "" {
		data, err := json.Marshal(home)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(path, data, 0o600); err != nil {
			t.Fatal(err)
		}
	}
	return home
}
