package store

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"errors"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"sync/atomic"
	"testing"
	"time"
)

const pngHeader = "\x89PNG\r\n\x1a\n"

// newTestImageCache trusts the test server and routes every dial to it.
func newTestImageCache(t *testing.T, handler http.HandlerFunc) (*ImageCache, *httptest.Server) {
	t.Helper()
	server := httptest.NewTLSServer(handler)
	t.Cleanup(server.Close)
	cache := newImageCache(t.TempDir(), func(ctx context.Context, network, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, server.Listener.Addr().String())
	})
	pool := x509.NewCertPool()
	pool.AddCert(server.Certificate())
	cache.http.Transport.(*http.Transport).TLSClientConfig = &tls.Config{RootCAs: pool}
	return cache, server
}

func TestImageCacheStoresAndReusesAnImage(t *testing.T) {
	var hits atomic.Int32
	cache, _ := newTestImageCache(t, func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		_, _ = w.Write([]byte(pngHeader + "payload"))
	})
	first, err := cache.Fetch(context.Background(), "https://example.com/a.png")
	if err != nil || first.ContentType != "image/png" || filepath.Ext(first.Path) != ".png" {
		t.Fatalf("first = %+v err=%v", first, err)
	}
	if info, err := os.Stat(first.Path); err != nil || info.Mode().Perm() != 0o600 {
		t.Fatalf("stat = %v err=%v", info, err)
	}
	second, err := cache.Fetch(context.Background(), "https://example.com/a.png")
	if err != nil || second != first || hits.Load() != 1 {
		t.Fatalf("second = %+v err=%v hits=%d", second, err, hits.Load())
	}
}

func TestImageCacheRejectsUnsafeInputs(t *testing.T) {
	cache, _ := newTestImageCache(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/text":
			_, _ = w.Write([]byte("<html>not an image</html>"))
		case "/big":
			_, _ = w.Write(append([]byte(pngHeader), make([]byte, maxImageBytes)...))
		case "/missing":
			http.NotFound(w, r)
		case "/redirect":
			http.Redirect(w, r, "http://example.com/plain.png", http.StatusFound)
		}
	})
	for _, raw := range []string{
		"http://example.com/a.png", "ftp://example.com/a.png", "https://user@example.com/a.png", "https:///a.png",
		"::", "https://example.com/text", "https://example.com/big", "https://example.com/missing", "https://example.com/redirect",
	} {
		if _, err := cache.Fetch(context.Background(), raw); err == nil {
			t.Errorf("%q was accepted", raw)
		}
	}
	entries, _ := os.ReadDir(cache.dir)
	for _, e := range entries {
		t.Errorf("rejected input left %s behind", e.Name())
	}
}

func TestPublicDialerRefusesNonPublicAddresses(t *testing.T) {
	dial := publicDialer()
	for _, addr := range []string{"127.0.0.1:443", "10.0.0.1:443", "192.168.1.1:443", "169.254.169.254:80", "[::1]:443", "0.0.0.0:80"} {
		ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
		conn, err := dial(ctx, "tcp", addr)
		cancel()
		if conn != nil {
			_ = conn.Close()
		}
		if !errors.Is(err, ErrImageRejected) {
			t.Errorf("%s: err = %v", addr, err)
		}
	}
}

func TestImageCacheEvictsLeastRecentlyUsedBeyondTheFileBound(t *testing.T) {
	cache := NewImageCache(t.TempDir())
	base := time.Now().Add(-time.Hour)
	for i := 0; i < maxImageFiles+3; i++ {
		path := filepath.Join(cache.dir, fmt.Sprintf("f%04d.png", i))
		if err := os.WriteFile(path, []byte("x"), 0o600); err != nil {
			t.Fatal(err)
		}
		mod := base.Add(time.Duration(i) * time.Second)
		if err := os.Chtimes(path, mod, mod); err != nil {
			t.Fatal(err)
		}
	}
	cache.evictLocked()
	entries, _ := os.ReadDir(cache.dir)
	if len(entries) != maxImageFiles {
		t.Fatalf("files = %d", len(entries))
	}
	if _, err := os.Stat(filepath.Join(cache.dir, "f0000.png")); !os.IsNotExist(err) {
		t.Fatalf("oldest file survived: %v", err)
	}
}
