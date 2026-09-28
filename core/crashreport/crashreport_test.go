package crashreport

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestParseDSN(t *testing.T) {
	dsn, err := ParseDSN("https://abc@o1.ingest.example/42")
	if err != nil || dsn.Key != "abc" || dsn.Project != "42" || dsn.Ingest != "https://o1.ingest.example" {
		t.Fatalf("dsn=%+v err=%v", dsn, err)
	}
	for _, bad := range []string{"", "http://abc@h/1", "https://h/1", "https://abc@h/", "https://abc@h/a/b"} {
		if _, err := ParseDSN(bad); err == nil {
			t.Errorf("accepted %q", bad)
		}
	}
}

func TestEnvelopeScrubsHome(t *testing.T) {
	body, err := Envelope(Report{Source: "client", Message: "boom at /Users/alice/x", Backtrace: "/Users/alice/y", Release: "1"}, "/Users/alice", time.Unix(0, 0))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(body), "alice") {
		t.Fatalf("home leaked: %s", body)
	}
	lines := strings.Split(strings.TrimSpace(string(body)), "\n")
	if len(lines) != 3 || !json.Valid([]byte(lines[2])) {
		t.Fatalf("bad envelope: %q", body)
	}
}

func TestUploadPostsEnvelope(t *testing.T) {
	var gotPath, gotAuth string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotPath, gotAuth = r.URL.Path, r.Header.Get("X-Sentry-Auth")
		_, _ = io.Copy(io.Discard, r.Body)
	}))
	defer server.Close()
	file := filepath.Join(t.TempDir(), "r.json")
	if err := os.WriteFile(file, []byte(`{"source":"core","message":"m","release":"1"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	if err := Upload(context.Background(), DSN{Key: "k", Ingest: server.URL, Project: "7"}, file, "", server.Client()); err != nil {
		t.Fatal(err)
	}
	if gotPath != "/api/7/envelope/" || !strings.Contains(gotAuth, "sentry_key=k") {
		t.Fatalf("path=%q auth=%q", gotPath, gotAuth)
	}
}
