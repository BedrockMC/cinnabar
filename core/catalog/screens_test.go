package catalog

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"github.com/sandertv/gophertunnel/minecraft/service/gatherings"
)

// Authored to the open-source catalog item shape; not a captured payload.
const featuredItemFixture = `{
	"Id": "item-1",
	"Title": {"NEUTRAL": "Example Network"},
	"Description": {"NEUTRAL": " A test server. "},
	"Tags": ["pvp"],
	"Images": [
		{"Id": "a", "Tag": "logo", "Type": "thumbnail", "Url": "https://cdn.example.test/logo.png"},
		{"Id": "b", "Tag": "shot", "Type": "screenshot", "Url": "https://cdn.example.test/shot.png"},
		{"Id": "c", "Tag": "game", "Type": "thumbnail", "Url": "http://insecure.example.test/game.png"}
	],
	"DisplayProperties": {
		"url": "play.example.test", "port": 19132, "creatorName": "Example",
		"news": " Season two ", "newsTitle": "News", "unknownField": [1, 2],
		"availableGames": [
			{"title": "Skywars", "subtitle": "Solo", "description": "Fight", "imageTag": "game"},
			{"title": "", "subtitle": ""}
		]
	}
}`

func parseFeatured(t *testing.T, raw string) *gatherings.FeaturedServer {
	t.Helper()
	var item playfabcatalog.Item
	if err := json.Unmarshal([]byte(raw), &item); err != nil {
		t.Fatal(err)
	}
	server, err := gatherings.NewClient(nil).ParseFeaturedServer(item)
	if err != nil {
		t.Fatal(err)
	}
	return server
}

func TestFeaturedServersCarryTheInfoPanelDetails(t *testing.T) {
	servers := featuredServers([]*gatherings.FeaturedServer{parseFeatured(t, featuredItemFixture), nil})
	if len(servers) != 1 {
		t.Fatalf("servers = %+v", servers)
	}
	server := servers[0]
	if server.Name != "Example Network" || server.Address != "play.example.test:19132" || server.Caption != "Skywars" {
		t.Fatalf("identity = %+v", server)
	}
	if server.Description != "A test server." || server.News != "Season two" || server.NewsTitle != "News" {
		t.Fatalf("text = %+v", server)
	}
	if server.Logo.URL != "https://cdn.example.test/logo.png" || len(server.Screenshots) != 1 {
		t.Fatalf("art = %+v", server)
	}
	if len(server.Games) != 1 || server.Games[0].Image.URL != "" {
		t.Fatalf("games keep only titled entries and HTTPS art: %+v", server.Games)
	}
}

func TestFeaturedServersSkipEntriesWithoutAnAddress(t *testing.T) {
	servers := featuredServers([]*gatherings.FeaturedServer{parseFeatured(t, `{"Id": "x", "DisplayProperties": {}}`)})
	if len(servers) != 0 {
		t.Fatalf("servers = %+v", servers)
	}
}

func TestArtworkPruningKeepsTheNewestFiles(t *testing.T) {
	directory := t.TempDir()
	for index, name := range []string{"a.img", "b.img", "c.img"} {
		path := filepath.Join(directory, name)
		if err := os.WriteFile(path, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
		stamp := time.Unix(int64(1000+index), 0)
		if err := os.Chtimes(path, stamp, stamp); err != nil {
			t.Fatal(err)
		}
	}
	pruneArtwork(directory, 2)
	if _, err := os.Stat(filepath.Join(directory, "a.img")); !os.IsNotExist(err) {
		t.Fatalf("the oldest file survived: %v", err)
	}
	if _, err := os.Stat(filepath.Join(directory, "c.img")); err != nil {
		t.Fatalf("the newest file was pruned: %v", err)
	}
}

func TestFeaturedImagesPointIntoTheServers(t *testing.T) {
	servers := []FeaturedServer{{Screenshots: []Image{{URL: "https://a.test/s.png"}}, Games: []Game{{}}}}
	images := FeaturedImages(servers)
	if len(images) != 3 {
		t.Fatalf("images = %d", len(images))
	}
	images[1].Path = "/cache/s.img"
	if servers[0].Screenshots[0].Path != "/cache/s.img" {
		t.Fatal("paths must land in the servers")
	}
}

// A gathering keeps one join address across refreshes until it goes stale.
func TestJoinMemoKeepsAnAddress(t *testing.T) {
	memo := &joinMemo{entries: map[string]joinEntry{}}
	joins := 0
	join := func() string {
		joins++
		return fmt.Sprintf("10.0.0.%d:19132", joins)
	}
	first := memo.lookup("exp", join)
	if again := memo.lookup("exp", join); again != first || joins != 1 {
		t.Fatalf("rejoined: %q then %q after %d joins", first, again, joins)
	}
	memo.entries["exp"] = joinEntry{address: first, at: time.Now().Add(-2 * joinTTL)}
	if fresh := memo.lookup("exp", join); fresh == first {
		t.Fatalf("stale address %q kept", fresh)
	}
	if empty := memo.lookup("down", func() string { return "" }); empty != "" {
		t.Fatalf("failed join gave %q", empty)
	}
}
