// Package store talks to Mojang's Marketplace services as the signed-in account: the store session,
// layout pages, catalog search, balance, inventory and Minecoin purchases.
package store

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/df-mc/go-playfab/v2"
	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"github.com/google/uuid"
	"github.com/sandertv/gophertunnel/minecraft/service"
)

const (
	pathConfig      = "/api/v1.0/session/config"
	pathBalances    = "/api/v1.0/currencies/virtual/balances"
	pathInventory   = "/api/v1.0/player/inventory?includeReceipt=true"
	pathRefresh     = "/api/v1.0/inventory/refresh"
	pathRowItems    = "/api/v2.0/layout/items"
	pathTransaction = "/api/v1.0/transaction/virtual"

	maxResponseBytes = 8 << 20
	requestTimeout   = 40 * time.Second
	configTTL        = 10 * time.Minute
	inventoryTTL     = time.Minute
	userAgent        = "libhttpclient/1.0.0.0"
)

// Catalog is the PlayFab catalog surface the store uses; *playfabcatalog.Client implements it.
type Catalog interface {
	SearchItems(ctx context.Context, filter playfabcatalog.SearchFilter, opts ...playfab.RequestOption) (*playfabcatalog.SearchResult, error)
	ItemByID(ctx context.Context, id string, opts ...playfab.RequestOption) (*playfabcatalog.Item, error)
}

// Identity is what the service tags every purchase with; fields left empty take defaults.
type Identity struct {
	XUID          string
	TitleID       string
	DeviceID      string // telemetry client id
	BuildPlatform int    // numeric build platform of the emulated client
	DNAPlatform   string
	EditionType   string
}

// Config wires a Client.
type Config struct {
	BaseURL  *url.URL
	Tokens   service.TokenSource
	Catalog  Catalog
	HTTP     *http.Client
	Identity Identity
	Now      func() time.Time
}

// Client is the store service client; it is safe for concurrent use.
type Client struct {
	cfg       Config
	sessionID string
	seq       atomic.Uint32
	guard     *purchaseGuard

	mu        sync.Mutex
	config    *SessionConfig
	configAt  time.Time
	inventory *inventoryCache
	etag      string // newest InventoryETag seen
	lists     string // newest X-UserLists-Version seen
}

// NewClient returns a Client; BaseURL, Tokens and Catalog are required.
func NewClient(cfg Config) (*Client, error) {
	if cfg.BaseURL == nil || !cfg.BaseURL.IsAbs() || cfg.BaseURL.Host == "" || cfg.Tokens == nil || cfg.Catalog == nil {
		return nil, errors.New("store: incomplete client configuration")
	}
	if cfg.HTTP == nil {
		cfg.HTTP = &http.Client{Timeout: requestTimeout}
	}
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	id := &cfg.Identity
	if id.DeviceID == "" {
		id.DeviceID = uuid.NewString()
	}
	if id.BuildPlatform == 0 {
		id.BuildPlatform = 7 // Windows 10, matching the device the auth token claims
	}
	if id.DNAPlatform == "" {
		id.DNAPlatform = service.PlatformWindows10
	}
	if id.EditionType == "" {
		id.EditionType = "Bedrock"
	}
	c := &Client{cfg: cfg, sessionID: uuid.NewString()}
	c.guard = newPurchaseGuard(cfg.Now)
	return c, nil
}

// ServiceError is a non-2xx answer from the store service; Body is bounded and never logged.
type ServiceError struct {
	Status int
	Body   []byte
}

func (e *ServiceError) Error() string {
	return fmt.Sprintf("store: service returned HTTP %d", e.Status)
}

// response is the decoded outcome of one service call.
type response struct {
	status int
	header http.Header
	body   []byte
}

// resolve joins a service-relative path (query allowed) onto the base URL, or accepts an https URL
// on the same host.
func (c *Client) resolve(path string) (string, error) {
	if strings.HasPrefix(path, "https://") {
		u, err := url.Parse(path)
		if err != nil || !strings.EqualFold(u.Host, c.cfg.BaseURL.Host) {
			return "", ErrInvalidRequest
		}
		return u.String(), nil
	}
	if !strings.HasPrefix(path, "/") {
		path = "/" + path
	}
	ref, err := url.Parse(path)
	if err != nil {
		return "", ErrInvalidRequest
	}
	return c.cfg.BaseURL.ResolveReference(ref).String(), nil
}

// do sends one authenticated request; it never retries, so a POST is sent at most once.
func (c *Client) do(ctx context.Context, method, path string, body any) (*response, error) {
	target, err := c.resolve(path)
	if err != nil {
		return nil, err
	}
	var reader io.Reader
	if body != nil {
		encoded, err := json.Marshal(body)
		if err != nil {
			return nil, fmt.Errorf("store: encode request: %w", err)
		}
		reader = bytes.NewReader(encoded)
	}
	req, err := http.NewRequestWithContext(ctx, method, target, reader)
	if err != nil {
		return nil, fmt.Errorf("store: make request: %w", err)
	}
	token, err := c.cfg.Tokens.ServiceToken(ctx)
	if err != nil {
		return nil, fmt.Errorf("store: service token: %w", err)
	}
	token.SetAuthHeader(req)
	req.Header.Set("Session-Id", c.sessionID)
	req.Header.Set("Accept", "application/json")
	req.Header.Set("User-Agent", userAgent)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.cfg.HTTP.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBytes+1))
	if err != nil {
		return nil, err
	}
	if len(data) > maxResponseBytes {
		return nil, errors.New("store: response too large")
	}
	out := &response{status: resp.StatusCode, header: resp.Header, body: data}
	if resp.StatusCode < 200 || resp.StatusCode > 299 {
		return out, &ServiceError{Status: resp.StatusCode, Body: data}
	}
	return out, nil
}

// result returns the envelope's "result" member.
func (r *response) result() (json.RawMessage, error) {
	var envelope struct {
		Result json.RawMessage `json:"result"`
	}
	if err := json.Unmarshal(r.body, &envelope); err != nil {
		return nil, fmt.Errorf("store: decode response: %w", err)
	}
	if len(envelope.Result) == 0 {
		return nil, errors.New("store: response has no result")
	}
	return envelope.Result, nil
}

// remember records the version headers the service attaches to inventory-aware answers.
func (c *Client) remember(h http.Header) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if v := h.Get("InventoryETag"); v != "" {
		c.etag = v
	}
	if v := h.Get("X-UserLists-Version"); v != "" {
		c.lists = v
	}
}
