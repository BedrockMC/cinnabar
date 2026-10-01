// Package store serves the launcher's Marketplace screens as the signed-in account: layout pages,
// catalog search, balance, inventory and confirmed Minecoin purchases. The store service protocol
// lives in gophertunnel's service/marketplace; this package keeps confirmation, purchase
// deduplication, caching and the bridge DTOs.
package store

import (
	"context"
	"errors"
	"sync"
	"sync/atomic"
	"time"

	"github.com/df-mc/go-playfab/v2"
	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"github.com/google/uuid"
	"github.com/sandertv/gophertunnel/minecraft/service"
	"github.com/sandertv/gophertunnel/minecraft/service/marketplace"
)

const (
	configTTL    = 10 * time.Minute
	inventoryTTL = time.Minute
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
	Market   *marketplace.Client
	Catalog  Catalog
	Identity Identity
	Now      func() time.Time
}

// Client is the store backend; it is safe for concurrent use.
type Client struct {
	cfg   Config
	seq   atomic.Uint32
	guard *purchaseGuard

	mu        sync.Mutex
	config    *marketplace.SessionConfig
	configAt  time.Time
	inventory *inventoryCache
	etag      string // newest inventory version seen
	lists     string // newest user-lists version seen
}

// NewClient returns a Client; Market and Catalog are required.
func NewClient(cfg Config) (*Client, error) {
	if cfg.Market == nil || cfg.Catalog == nil {
		return nil, errors.New("store: incomplete client configuration")
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
	return &Client{cfg: cfg, guard: newPurchaseGuard(cfg.Now)}, nil
}
