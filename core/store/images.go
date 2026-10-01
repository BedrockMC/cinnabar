package store

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"
	"time"
)

const (
	maxImageBytes   = 4 << 20
	maxImageFiles   = 512
	maxImageDirSize = 256 << 20
	imageTimeout    = 20 * time.Second
	maxRedirects    = 3
)

// ErrImageRejected is returned for a URL or payload the image cache refuses.
var ErrImageRejected = errors.New("store: image rejected")

// Image is a cached offer image on local disk.
type Image struct {
	Path        string `json:"path"`
	ContentType string `json:"content_type"`
}

// ImageCache downloads offer images over https into a bounded directory; it refuses local and private
// addresses and anything that is not a PNG, JPEG, GIF or BMP.
type ImageCache struct {
	dir  string
	http *http.Client

	mu sync.Mutex
}

var imageExtensions = map[string]string{"image/png": ".png", "image/jpeg": ".jpg", "image/gif": ".gif", "image/bmp": ".bmp"}

// NewImageCache returns a cache rooted at dir, created on first use.
func NewImageCache(dir string) *ImageCache {
	return newImageCache(dir, publicDialer())
}

func newImageCache(dir string, dial func(ctx context.Context, network, addr string) (net.Conn, error)) *ImageCache {
	transport := &http.Transport{DialContext: dial, TLSHandshakeTimeout: 10 * time.Second, DisableKeepAlives: true}
	client := &http.Client{
		Transport: transport, Timeout: imageTimeout,
		CheckRedirect: func(req *http.Request, via []*http.Request) error {
			if len(via) > maxRedirects || req.URL.Scheme != "https" {
				return ErrImageRejected
			}
			return nil
		},
	}
	return &ImageCache{dir: dir, http: client}
}

// publicDialer resolves the host itself and connects only to globally routable addresses.
func publicDialer() func(ctx context.Context, network, addr string) (net.Conn, error) {
	dialer := &net.Dialer{Timeout: 10 * time.Second}
	return func(ctx context.Context, network, addr string) (net.Conn, error) {
		host, port, err := net.SplitHostPort(addr)
		if err != nil {
			return nil, ErrImageRejected
		}
		ips, err := net.DefaultResolver.LookupIPAddr(ctx, host)
		if err != nil || len(ips) == 0 {
			return nil, ErrImageRejected
		}
		for _, ip := range ips {
			if !publicIP(ip.IP) {
				return nil, ErrImageRejected
			}
		}
		return dialer.DialContext(ctx, network, net.JoinHostPort(ips[0].IP.String(), port))
	}
}

func publicIP(ip net.IP) bool {
	return ip.IsGlobalUnicast() && !ip.IsPrivate() && !ip.IsLoopback() && !ip.IsLinkLocalUnicast() && !ip.IsUnspecified()
}

// Fetch returns the cached image for rawURL, downloading it when absent.
func (c *ImageCache) Fetch(ctx context.Context, rawURL string) (Image, error) {
	u, err := url.Parse(rawURL)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" || u.User != nil || len(rawURL) > 1024 {
		return Image{}, ErrImageRejected
	}
	sum := sha256.Sum256([]byte(u.String()))
	stem := filepath.Join(c.dir, hex.EncodeToString(sum[:]))
	for _, ext := range imageExtensions {
		if _, err := os.Stat(stem + ext); err == nil {
			now := time.Now()
			_ = os.Chtimes(stem+ext, now, now)
			return Image{Path: stem + ext, ContentType: contentTypeFor(ext)}, nil
		}
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, u.String(), nil)
	if err != nil {
		return Image{}, ErrImageRejected
	}
	req.Header.Set("User-Agent", "libhttpclient/1.0.0.0")
	resp, err := c.http.Do(req)
	if err != nil {
		return Image{}, fmt.Errorf("store: fetch image: %w", err)
	}
	defer resp.Body.Close()
	if resp.StatusCode != http.StatusOK {
		return Image{}, ErrImageRejected
	}
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxImageBytes+1))
	if err != nil {
		return Image{}, fmt.Errorf("store: read image: %w", err)
	}
	if len(data) > maxImageBytes {
		return Image{}, ErrImageRejected
	}
	contentType := sniffImage(data)
	ext, ok := imageExtensions[contentType]
	if !ok {
		return Image{}, ErrImageRejected
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if err := os.MkdirAll(c.dir, 0o700); err != nil {
		return Image{}, fmt.Errorf("store: image cache: %w", err)
	}
	temp, err := os.CreateTemp(c.dir, ".image-*")
	if err != nil {
		return Image{}, fmt.Errorf("store: image cache: %w", err)
	}
	name := temp.Name()
	_, writeErr := temp.Write(data)
	closeErr := temp.Close()
	if err := errors.Join(writeErr, closeErr); err != nil {
		_ = os.Remove(name)
		return Image{}, fmt.Errorf("store: image cache: %w", err)
	}
	if err := os.Chmod(name, 0o600); err != nil {
		_ = os.Remove(name)
		return Image{}, fmt.Errorf("store: image cache: %w", err)
	}
	if err := os.Rename(name, stem+ext); err != nil {
		_ = os.Remove(name)
		return Image{}, fmt.Errorf("store: image cache: %w", err)
	}
	c.evictLocked()
	return Image{Path: stem + ext, ContentType: contentType}, nil
}

func sniffImage(data []byte) string {
	switch {
	case len(data) >= 8 && string(data[:8]) == "\x89PNG\r\n\x1a\n":
		return "image/png"
	case len(data) >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF:
		return "image/jpeg"
	case len(data) >= 6 && (string(data[:6]) == "GIF87a" || string(data[:6]) == "GIF89a"):
		return "image/gif"
	case len(data) >= 2 && data[0] == 'B' && data[1] == 'M':
		return "image/bmp"
	}
	return ""
}

func contentTypeFor(ext string) string {
	for contentType, e := range imageExtensions {
		if e == ext {
			return contentType
		}
	}
	return ""
}

// evictLocked removes the least recently used files beyond the count and size bounds.
func (c *ImageCache) evictLocked() {
	entries, err := os.ReadDir(c.dir)
	if err != nil {
		return
	}
	type file struct {
		path string
		size int64
		mod  time.Time
	}
	var files []file
	var total int64
	for _, entry := range entries {
		info, err := entry.Info()
		if err != nil || !info.Mode().IsRegular() || strings.HasPrefix(entry.Name(), ".") {
			continue
		}
		files = append(files, file{filepath.Join(c.dir, entry.Name()), info.Size(), info.ModTime()})
		total += info.Size()
	}
	sort.Slice(files, func(i, j int) bool { return files[i].mod.Before(files[j].mod) })
	for len(files) > maxImageFiles || total > maxImageDirSize {
		if len(files) == 0 {
			return
		}
		_ = os.Remove(files[0].path)
		total -= files[0].size
		files = files[1:]
	}
}
