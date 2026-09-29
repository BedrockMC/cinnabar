package store

import (
	"context"
	"encoding/json"
	"errors"
	"regexp"
	"strings"
	"time"
)

const (
	maxKnownPages     = 256
	maxInventoryIDs   = 20000
	maxEntitlementWin = 800
)

// SessionConfig is the store's per-session configuration; unknown members are ignored.
type SessionConfig struct {
	KnownPages     map[string]string
	TextureVersion string
}

type inventoryCache struct {
	ids     []string
	set     map[string]struct{}
	version string
	at      time.Time
}

var pagePattern = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,64}$`)

// flexInt decodes a JSON number or numeric string.
type flexInt int64

func (f *flexInt) UnmarshalJSON(b []byte) error {
	var n json.Number
	if len(b) > 0 && b[0] == '"' {
		var s string
		if err := json.Unmarshal(b, &s); err != nil {
			return err
		}
		n = json.Number(strings.TrimSpace(s))
	} else if err := json.Unmarshal(b, &n); err != nil {
		return err
	}
	if v, err := n.Int64(); err == nil {
		*f = flexInt(v)
		return nil
	}
	v, err := n.Float64()
	if err != nil {
		return err
	}
	*f = flexInt(v)
	return nil
}

func parseSessionConfig(result json.RawMessage) SessionConfig {
	var raw struct {
		KnownPages     map[string]string `json:"knownPages"`
		TextureVersion string            `json:"latestTextureVersion"`
	}
	// A malformed member leaves its zero value; the rest of the config stays usable.
	_ = json.Unmarshal(result, &raw)
	cfg := SessionConfig{TextureVersion: raw.TextureVersion, KnownPages: map[string]string{}}
	for name, path := range raw.KnownPages {
		if len(cfg.KnownPages) >= maxKnownPages {
			break
		}
		cfg.KnownPages[name] = path
	}
	return cfg
}

// SessionConfig returns the store session configuration, cached briefly.
func (c *Client) SessionConfig(ctx context.Context) (SessionConfig, error) {
	c.mu.Lock()
	if c.config != nil && c.cfg.Now().Sub(c.configAt) < configTTL {
		cfg := *c.config
		c.mu.Unlock()
		return cfg, nil
	}
	c.mu.Unlock()
	resp, err := c.do(ctx, "GET", pathConfig, nil)
	if err != nil {
		return SessionConfig{}, err
	}
	result, err := resp.result()
	if err != nil {
		return SessionConfig{}, err
	}
	cfg := parseSessionConfig(result)
	c.mu.Lock()
	c.config, c.configAt = &cfg, c.cfg.Now()
	c.mu.Unlock()
	return cfg, nil
}

func parseBalances(result json.RawMessage) []Balance {
	var raw struct {
		Balances []struct {
			Type   string  `json:"type"`
			Amount flexInt `json:"amount"`
		} `json:"virtualCurrencyBalances"`
	}
	_ = json.Unmarshal(result, &raw)
	out := make([]Balance, 0, len(raw.Balances))
	for _, b := range raw.Balances {
		if b.Type != "" {
			out = append(out, Balance{Currency: b.Type, Amount: int64(b.Amount)})
		}
	}
	return out
}

// Balances returns the account's virtual currency balances (Minecoins among them).
func (c *Client) Balances(ctx context.Context) ([]Balance, error) {
	resp, err := c.do(ctx, "GET", pathBalances, nil)
	if err != nil {
		return nil, err
	}
	c.remember(resp.header)
	result, err := resp.result()
	if err != nil {
		return nil, err
	}
	return parseBalances(result), nil
}

var inventoryListKeys = map[string]struct{}{"entitlements": {}, "items": {}, "inventory": {}, "owned": {}}

// parseInventoryIDs collects owned content ids from the inventory result; an entry is an id string or
// an object carrying one of the known id members.
func parseInventoryIDs(result json.RawMessage) []string {
	var top any
	if json.Unmarshal(result, &top) != nil {
		return nil
	}
	var list []any
	switch v := top.(type) {
	case []any:
		list = v
	case map[string]any:
		for key, member := range v {
			if _, ok := inventoryListKeys[strings.ToLower(key)]; ok {
				if arr, ok := member.([]any); ok {
					list = append(list, arr...)
				}
			}
		}
	}
	seen := map[string]struct{}{}
	ids := make([]string, 0, len(list))
	for _, entry := range list {
		id := entryID(entry)
		if id == "" || !ValidOfferID(id) {
			continue
		}
		if _, dup := seen[id]; dup {
			continue
		}
		seen[id] = struct{}{}
		ids = append(ids, strings.ToLower(id))
		if len(ids) >= maxInventoryIDs {
			break
		}
	}
	return ids
}

func entryID(entry any) string {
	switch v := entry.(type) {
	case string:
		return v
	case map[string]any:
		for _, key := range []string{"id", "itemId", "offerId", "productId", "entitlementId"} {
			if s, ok := v[key].(string); ok && s != "" {
				return s
			}
		}
	}
	return ""
}

func (c *Client) loadInventory(ctx context.Context, force bool) (*inventoryCache, error) {
	c.mu.Lock()
	if cached := c.inventory; !force && cached != nil && c.cfg.Now().Sub(cached.at) < inventoryTTL {
		c.mu.Unlock()
		return cached, nil
	}
	c.mu.Unlock()
	resp, err := c.do(ctx, "GET", pathInventory, nil)
	if err != nil {
		return nil, err
	}
	c.remember(resp.header)
	result, err := resp.result()
	if err != nil {
		return nil, err
	}
	ids := parseInventoryIDs(result)
	fresh := &inventoryCache{ids: ids, set: make(map[string]struct{}, len(ids)), at: c.cfg.Now()}
	for _, id := range ids {
		fresh.set[id] = struct{}{}
	}
	c.mu.Lock()
	fresh.version = c.etag
	c.inventory = fresh
	c.mu.Unlock()
	return fresh, nil
}

func (c *Client) invalidateInventory() {
	c.mu.Lock()
	c.inventory = nil
	c.mu.Unlock()
}

// Entitlements returns a window of the owned content ids; refresh re-reads them from the service
// instead of the short-lived cache.
func (c *Client) Entitlements(ctx context.Context, offset, limit int, refresh bool) (Entitlements, error) {
	if offset < 0 || limit < 0 {
		return Entitlements{}, ErrInvalidRequest
	}
	if limit == 0 || limit > maxEntitlementWin {
		limit = maxEntitlementWin
	}
	if refresh && offset == 0 {
		c.RefreshInventory(ctx)
	}
	inv, err := c.loadInventory(ctx, refresh && offset == 0)
	if err != nil {
		return Entitlements{}, err
	}
	out := Entitlements{Total: len(inv.ids), Offset: offset, InventoryVersion: inv.version, Owned: []string{}}
	if offset < len(inv.ids) {
		end := min(offset+limit, len(inv.ids))
		out.Owned = append(out.Owned, inv.ids[offset:end]...)
	}
	return out, nil
}

// RefreshInventory asks the service to refresh the account's inventory version. It is best effort:
// a failure leaves the next inventory read to return whatever the service has.
func (c *Client) RefreshInventory(ctx context.Context) {
	resp, err := c.do(ctx, "POST", pathRefresh, struct{}{})
	if err != nil {
		return
	}
	c.remember(resp.header)
	c.invalidateInventory()
}

// MoreOffers loads the next slice of a row from a continuation token the page handed out.
func (c *Client) MoreOffers(ctx context.Context, token string) (RowMore, error) {
	if !ValidContinuation(token) {
		return RowMore{}, ErrInvalidRequest
	}
	if _, err := c.loadInventory(ctx, false); err != nil {
		return RowMore{}, err
	}
	c.mu.Lock()
	version := c.etag
	c.mu.Unlock()
	resp, err := c.do(ctx, "POST", pathRowItems, layoutMoreBody{ContinuationToken: token, InventoryVersion: version})
	if err != nil {
		return RowMore{}, err
	}
	c.remember(resp.header)
	result, err := resp.result()
	if err != nil {
		return RowMore{}, err
	}
	more := parseRowMore(result)
	for i := range more.Offers {
		more.Offers[i].Owned = c.owned(more.Offers[i].ID)
	}
	return more, nil
}

// owned reports whether the offer id is in the cached inventory; false when the inventory is unknown.
func (c *Client) owned(id string) bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.inventory == nil {
		return false
	}
	_, ok := c.inventory.set[strings.ToLower(id)]
	return ok
}

// markOwned annotates offers from the inventory, loading it when absent; a failed load leaves them unowned.
func (c *Client) markOwned(ctx context.Context, offers []Offer) {
	if _, err := c.loadInventory(ctx, false); err != nil {
		return
	}
	for i := range offers {
		offers[i].Owned = c.owned(offers[i].ID)
	}
}

// layoutBody is the request body of a layout page call.
type layoutBody struct {
	Entitlements     []string `json:"entitlements"`
	InventoryVersion string   `json:"inventoryVersion"`
	ListVersion      string   `json:"listVersion"`
}

// layoutMoreBody is the request body of a row continuation call.
type layoutMoreBody struct {
	ContinuationToken string `json:"continuationToken"`
	InventoryVersion  string `json:"inventoryVersion"`
}

// Home loads a known store page by its session-config name and reduces it to rows of offers.
func (c *Client) Home(ctx context.Context, page string) (Page, error) {
	if !pagePattern.MatchString(page) {
		return Page{}, ErrInvalidRequest
	}
	cfg, err := c.SessionConfig(ctx)
	if err != nil {
		return Page{}, err
	}
	path, ok := cfg.KnownPages[page]
	if !ok || path == "" {
		return Page{}, ErrUnknownPage
	}
	inv, err := c.loadInventory(ctx, false)
	if err != nil {
		return Page{}, err
	}
	c.mu.Lock()
	body := layoutBody{Entitlements: inv.ids, InventoryVersion: c.etag, ListVersion: c.lists}
	c.mu.Unlock()
	resp, err := c.do(ctx, "POST", path, body)
	if err != nil {
		return Page{}, err
	}
	c.remember(resp.header)
	result, err := resp.result()
	if err != nil {
		return Page{}, err
	}
	out := parsePage(page, result)
	c.mu.Lock()
	out.InventoryVersion = c.etag
	c.mu.Unlock()
	for i := range out.Rows {
		for j := range out.Rows[i].Offers {
			out.Rows[i].Offers[j].Owned = c.owned(out.Rows[i].Offers[j].ID)
		}
	}
	return out, nil
}

var errNoOffer = errors.New("store: offer not found")
