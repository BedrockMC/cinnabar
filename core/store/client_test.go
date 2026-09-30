package store

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/df-mc/go-playfab/v2"
	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"github.com/sandertv/gophertunnel/minecraft/service"
)

type fakeTokens struct{}

func (fakeTokens) ServiceToken(context.Context) (*service.Token, error) {
	return &service.Token{AuthorizationHeader: "MCToken synthetic"}, nil
}

type fakeCatalog struct {
	items []playfabcatalog.Item
	got   playfabcatalog.SearchFilter
}

func (f *fakeCatalog) SearchItems(_ context.Context, filter playfabcatalog.SearchFilter, _ ...playfab.RequestOption) (*playfabcatalog.SearchResult, error) {
	f.got = filter
	return &playfabcatalog.SearchResult{Items: f.items, ContinuationToken: "next"}, nil
}

func (f *fakeCatalog) ItemByID(_ context.Context, id string, _ ...playfab.RequestOption) (*playfabcatalog.Item, error) {
	for i := range f.items {
		if f.items[i].ID == id {
			return &f.items[i], nil
		}
	}
	return nil, errors.New("missing")
}

// newTestClient serves handler on a loopback server and returns a Client pointed at it.
func newTestClient(t *testing.T, handler http.HandlerFunc, cat Catalog) *Client {
	t.Helper()
	server := httptest.NewServer(handler)
	t.Cleanup(server.Close)
	base, err := url.Parse(server.URL)
	if err != nil {
		t.Fatal(err)
	}
	if cat == nil {
		cat = &fakeCatalog{}
	}
	client, err := NewClient(Config{BaseURL: base, Tokens: fakeTokens{}, Catalog: cat, Identity: Identity{XUID: "2535", TitleID: "20CA2"}})
	if err != nil {
		t.Fatal(err)
	}
	return client
}

const inventoryFixture = `{"result":{"entitlements":[{"id":"AAAAAAAA-0000-0000-0000-000000000001"},{"itemId":"bbbbbbbb-0000-0000-0000-000000000002"},"cccccccc-0000-0000-0000-000000000003",{"nothing":1},{"id":"bad id"}]}}`

func TestBalancesAcceptNumberAndStringAmounts(t *testing.T) {
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != pathBalances || r.Header.Get("Authorization") != "MCToken synthetic" || r.Header.Get("Session-Id") == "" {
			t.Errorf("request = %s %v", r.URL.Path, r.Header)
		}
		_, _ = io.WriteString(w, `{"result":{"virtualCurrencyBalances":[{"type":"mc","amount":"1500"},{"type":"pt","amount":7},{"amount":3}]}}`)
	}, nil)
	balances, err := client.Balances(context.Background())
	if err != nil || len(balances) != 2 || balances[0] != (Balance{"mc", 1500}) || balances[1] != (Balance{"pt", 7}) {
		t.Fatalf("balances = %+v err=%v", balances, err)
	}
}

func TestEntitlementsAreLenientAndPaged(t *testing.T) {
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("InventoryETag", "etag-1")
		_, _ = io.WriteString(w, inventoryFixture)
	}, nil)
	all, err := client.Entitlements(context.Background(), 0, 0, false)
	if err != nil || all.Total != 3 || len(all.Owned) != 3 || all.InventoryVersion != "etag-1" || all.Owned[0] != "aaaaaaaa-0000-0000-0000-000000000001" {
		t.Fatalf("entitlements = %+v err=%v", all, err)
	}
	window, err := client.Entitlements(context.Background(), 2, 5, false)
	if err != nil || len(window.Owned) != 1 || window.Offset != 2 {
		t.Fatalf("window = %+v err=%v", window, err)
	}
	if _, err := client.Entitlements(context.Background(), -1, 0, false); !errors.Is(err, ErrInvalidRequest) {
		t.Fatalf("negative offset err = %v", err)
	}
}

func TestHomeSendsOwnershipAndParsesRows(t *testing.T) {
	var layoutBodies []layoutBody
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case pathConfig:
			_, _ = io.WriteString(w, `{"result":{"knownPages":{"store":"/api/v2.0/layout/store"},"latestTextureVersion":"7","future":{"x":1}}}`)
		case pathInventory[:strings.Index(pathInventory, "?")]:
			w.Header().Set("InventoryETag", "etag-9")
			_, _ = io.WriteString(w, inventoryFixture)
		case "/api/v2.0/layout/store":
			var body layoutBody
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil || r.Method != http.MethodPost {
				t.Errorf("layout request: %v %s", err, r.Method)
			}
			layoutBodies = append(layoutBodies, body)
			_, _ = io.WriteString(w, layoutFixture)
		default:
			http.NotFound(w, r)
		}
	}, nil)
	page, err := client.Home(context.Background(), "store")
	if err != nil {
		t.Fatal(err)
	}
	if len(layoutBodies) != 1 || len(layoutBodies[0].Entitlements) != 3 || layoutBodies[0].InventoryVersion != "etag-9" {
		t.Fatalf("layout body = %+v", layoutBodies)
	}
	if len(page.Rows) != 2 || page.Rows[0].Title != "Featured" || len(page.Rows[0].Offers) != 2 {
		t.Fatalf("rows = %+v", page.Rows)
	}
	first := page.Rows[0].Offers[0]
	if first.ID != "aaaaaaaa-0000-0000-0000-000000000001" || !first.Owned || first.StoreID != "store-1" ||
		first.ThumbnailURL != "https://cdn.example.test/a.png" || len(first.Prices) != 1 || first.Prices[0].Amount != 320 {
		t.Fatalf("first offer = %+v", first)
	}
	second := page.Rows[0].Offers[1]
	if second.Owned || second.Title != "Neutral Name" || second.Prices[0] != (Price{"mc", 640}) {
		t.Fatalf("second offer = %+v", second)
	}
	if _, err := client.Home(context.Background(), "nope"); !errors.Is(err, ErrUnknownPage) {
		t.Fatalf("unknown page err = %v", err)
	}
	if _, err := client.Home(context.Background(), "../etc"); !errors.Is(err, ErrInvalidRequest) {
		t.Fatalf("bad page err = %v", err)
	}
}

const layoutFixture = `{"result":{"page":{"sections":[
 {"rows":[
  {"id":"r1","title":"Featured","storeId":"store-1","offers":[
    {"id":"AAAAAAAA-0000-0000-0000-000000000001","title":"Alpha","thumbnail":{"url":"https://cdn.example.test/a.png"},"price":320},
    {"id":"dddddddd-0000-0000-0000-000000000004","title":{"NEUTRAL":"Neutral Name","en-GB":"Other"},"prices":[{"currency":"mc","amount":"640"}],"image":"http://insecure.example.test/x.png"}]},
  {"id":"r2","title":{"neutral":"Second"},"items":[{"id":"eeeeeeee-0000-0000-0000-000000000005","name":"Echo","tags":["a","b"]},{"title":"no id"}]}
 ]}]}}}`

func TestParsePageBoundsAndSkipsUnknownShapes(t *testing.T) {
	var rows []string
	for i := 0; i < 90; i++ {
		rows = append(rows, `{"title":"R","offers":[{"id":"o`+strings.Repeat("x", 3)+`","title":"T"},{"id":"o2","title":"T"}]}`)
	}
	page := parsePage("store", json.RawMessage(`{"rows":[`+strings.Join(rows, ",")+`]}`))
	if !page.Truncated || len(page.Rows) > maxPageRows {
		t.Fatalf("truncated=%v rows=%d", page.Truncated, len(page.Rows))
	}
	total := 0
	for _, row := range page.Rows {
		total += len(row.Offers)
	}
	if total > maxPageOffers {
		t.Fatalf("offers = %d", total)
	}
	for _, junk := range []string{`null`, `[]`, `"x"`, `{"rows":"no"}`, `{"page":{"rows":[1,2,{"offers":[3]}]}}`} {
		if got := parsePage("p", json.RawMessage(junk)); len(got.Rows) != 0 {
			t.Fatalf("junk %s produced rows %+v", junk, got.Rows)
		}
	}
}

func TestSearchMapsCatalogItemsAndMarksOwned(t *testing.T) {
	cat := &fakeCatalog{items: []playfabcatalog.Item{
		{
			ID: "aaaaaaaa-0000-0000-0000-000000000001", ContentType: "MarketplaceDurableCatalog_V1.2",
			Title:             playfabcatalog.Dictionary[string]{"NEUTRAL": "Alpha"},
			DisplayProperties: json.RawMessage(`{"creatorName":"Studio"}`),
			Images:            []playfabcatalog.Image{{Type: "screenshot", URL: "https://cdn.example.test/s.png"}, {Type: "Thumbnail", URL: "https://cdn.example.test/t.png"}},
			PriceOptions:      playfabcatalog.PriceOptions{{Amounts: []playfabcatalog.PriceAmount{{Value: 320, ItemID: "mc"}}}},
			Rating:            playfabcatalog.Rating{Average: 4.5, TotalCount: 10},
		},
		{ID: "hidden", Hidden: true, Title: playfabcatalog.Dictionary[string]{"NEUTRAL": "H"}},
		{ID: "untitled"},
	}}
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) { _, _ = io.WriteString(w, inventoryFixture) }, cat)
	results, err := client.Search(context.Background(), SearchQuery{Term: "castle"})
	if err != nil || len(results.Offers) != 1 || results.Continuation != "next" {
		t.Fatalf("results = %+v err=%v", results, err)
	}
	offer := results.Offers[0]
	if !offer.Owned || offer.Creator != "Studio" || offer.ThumbnailURL != "https://cdn.example.test/t.png" ||
		offer.Prices[0] != (Price{"mc", 320}) || offer.Rating == nil || offer.Rating.Count != 10 {
		t.Fatalf("offer = %+v", offer)
	}
	if cat.got.Count != defaultSearchCount || cat.got.Term != "castle" {
		t.Fatalf("filter = %+v", cat.got)
	}
	detail, err := client.Offer(context.Background(), "aaaaaaaa-0000-0000-0000-000000000001")
	if err != nil || len(detail.ScreenshotURLs) != 1 || !detail.Owned {
		t.Fatalf("detail = %+v err=%v", detail, err)
	}
	if _, err := client.Search(context.Background(), SearchQuery{Filter: "a;drop"}); !errors.Is(err, ErrInvalidRequest) {
		t.Fatalf("bad filter err = %v", err)
	}
}

type purchaseServer struct {
	calls     atomic.Int32
	refreshes atomic.Int32
	bodies    []purchaseBody
	mu        sync.Mutex
	status    int
	header    map[string]string
	body      string
	gate      chan struct{}
}

func (s *purchaseServer) handler(t *testing.T) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == pathRefresh {
			s.refreshes.Add(1)
			_, _ = io.WriteString(w, `{"result":{"version":"v2"}}`)
			return
		}
		if r.URL.Path != pathTransaction || r.Method != http.MethodPost {
			t.Errorf("unexpected %s %s", r.Method, r.URL.Path)
		}
		s.calls.Add(1)
		var body purchaseBody
		_ = json.NewDecoder(r.Body).Decode(&body)
		s.mu.Lock()
		s.bodies = append(s.bodies, body)
		s.mu.Unlock()
		if s.gate != nil {
			<-s.gate
		}
		for k, v := range s.header {
			w.Header().Set(k, v)
		}
		w.WriteHeader(s.status)
		_, _ = io.WriteString(w, s.body)
	}
}

func request(id, offer string) PurchaseRequest {
	return PurchaseRequest{PurchaseID: id, OfferID: offer, StoreID: "store-1", Currency: "mc", Amount: "320", Confirmed: true}
}

func TestPurchaseSendsTheVanillaRequestShape(t *testing.T) {
	server := &purchaseServer{status: 200, header: map[string]string{"InventoryEtag": "etag-2"}, body: `{"result":{}}`}
	client := newTestClient(t, server.handler(t), nil)
	res, err := client.Purchase(context.Background(), request("purchase-0000000001", "offer-1"))
	if err != nil || res.Status != PurchaseOK || res.InventoryVersion != "etag-2" || res.CorrelationID == "" {
		t.Fatalf("result = %+v err=%v", res, err)
	}
	raw, _ := json.Marshal(server.bodies[0])
	for _, want := range []string{
		`"VirtualCurrency":{"Type":"mc","Amount":"320"}`, `"OfferId":"offer-1"`, `"StoreId":"store-1"`,
		`"TitleId":"20CA2"`, `"BuildPlat":7`, `"Xuid":"2535"`, `"Seq":1`, `"CorrelationId":"` + res.CorrelationID + `"`,
	} {
		if !strings.Contains(string(raw), want) {
			t.Errorf("body %s lacks %s", raw, want)
		}
	}
	if strings.Contains(string(raw), "UnitDurationInSeconds") {
		t.Errorf("body carries a duration it was not given: %s", raw)
	}
}

func TestPurchaseIsIdempotentPerID(t *testing.T) {
	server := &purchaseServer{status: 200, body: `{"result":{}}`}
	client := newTestClient(t, server.handler(t), nil)
	req := request("purchase-0000000002", "offer-2")
	first, err := client.Purchase(context.Background(), req)
	if err != nil || first.Replayed {
		t.Fatalf("first = %+v err=%v", first, err)
	}
	again, err := client.Purchase(context.Background(), req)
	if err != nil || !again.Replayed || again.CorrelationID != first.CorrelationID || server.calls.Load() != 1 {
		t.Fatalf("replay = %+v err=%v calls=%d", again, err, server.calls.Load())
	}
	changed := req
	changed.Amount = "1"
	if _, err := client.Purchase(context.Background(), changed); !errors.Is(err, ErrPurchaseReused) {
		t.Fatalf("changed replay err = %v", err)
	}
}

func TestPurchaseLocksTheOfferWhileInFlight(t *testing.T) {
	server := &purchaseServer{status: 200, body: `{"result":{}}`, gate: make(chan struct{})}
	client := newTestClient(t, server.handler(t), nil)
	done := make(chan error, 1)
	go func() {
		_, err := client.Purchase(context.Background(), request("purchase-0000000003", "offer-3"))
		done <- err
	}()
	deadline := time.Now().Add(5 * time.Second)
	for server.calls.Load() == 0 && time.Now().Before(deadline) {
		time.Sleep(time.Millisecond)
	}
	if _, err := client.Purchase(context.Background(), request("purchase-0000000004", "offer-3")); !errors.Is(err, ErrPurchaseBusy) {
		t.Fatalf("second purchase err = %v", err)
	}
	close(server.gate)
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	if server.calls.Load() != 1 {
		t.Fatalf("calls = %d", server.calls.Load())
	}
}

func TestPurchaseOutcomesFollowTheHTTPStatus(t *testing.T) {
	for _, test := range []struct {
		status int
		body   string
		want   string
		code   int
	}{
		{422, `{"code":"PlayFabError","customData":{"marketplaceErrorCode":"1234"}}`, PurchasePriceRefused, 1234},
		{412, `{}`, PurchaseStaleState, defaultMarketCode},
		{500, `not json`, PurchaseFailed, defaultMarketCode},
		{400, `{"code":"Other","customData":{"marketplaceErrorCode":5}}`, PurchaseFailed, defaultMarketCode},
	} {
		server := &purchaseServer{status: test.status, body: test.body}
		client := newTestClient(t, server.handler(t), nil)
		res, err := client.Purchase(context.Background(), request("purchase-0000000005", "offer-5"))
		if err != nil || res.Status != test.want || res.MarketplaceErrorCode != test.code || res.HTTPStatus != test.status {
			t.Fatalf("status %d: result = %+v err=%v", test.status, res, err)
		}
		// A definitive refusal releases the offer for a fresh attempt.
		if _, err := client.Purchase(context.Background(), request("purchase-0000000006", "offer-5")); errors.Is(err, ErrPurchaseBusy) {
			t.Fatalf("status %d left the offer locked", test.status)
		}
	}
}

func TestPurchaseWithNoAnswerIsUnknownAndHoldsTheOffer(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {}))
	base, _ := url.Parse(server.URL)
	server.Close() // connection refused: the outcome cannot be known to have missed the service
	client, err := NewClient(Config{BaseURL: base, Tokens: fakeTokens{}, Catalog: &fakeCatalog{}})
	if err != nil {
		t.Fatal(err)
	}
	res, err := client.Purchase(context.Background(), request("purchase-0000000007", "offer-7"))
	if err != nil || res.Status != PurchaseUnknown {
		t.Fatalf("result = %+v err=%v", res, err)
	}
	if _, err := client.Purchase(context.Background(), request("purchase-0000000008", "offer-7")); !errors.Is(err, ErrPurchaseBusy) {
		t.Fatalf("retry of an unknown outcome err = %v", err)
	}
}

func TestPurchaseValidationRefusesUnconfirmedOrMalformedRequests(t *testing.T) {
	server := &purchaseServer{status: 200, body: `{"result":{}}`}
	client := newTestClient(t, server.handler(t), nil)
	base := request("purchase-0000000009", "offer-9")
	mutations := map[string]func(*PurchaseRequest){
		"unconfirmed": func(r *PurchaseRequest) { r.Confirmed = false },
		"zero amount": func(r *PurchaseRequest) { r.Amount = "0" },
		"negative":    func(r *PurchaseRequest) { r.Amount = "-5" },
		"decimal":     func(r *PurchaseRequest) { r.Amount = "3.5" },
		"short id":    func(r *PurchaseRequest) { r.PurchaseID = "abc" },
		"bad offer":   func(r *PurchaseRequest) { r.OfferID = "a b" },
		"no currency": func(r *PurchaseRequest) { r.Currency = "" },
	}
	for name, mutate := range mutations {
		r := base
		mutate(&r)
		if _, err := client.Purchase(context.Background(), r); !errors.Is(err, ErrInvalidRequest) {
			t.Errorf("%s: err = %v", name, err)
		}
	}
	if server.calls.Load() != 0 {
		t.Fatalf("invalid purchases reached the service %d times", server.calls.Load())
	}
}

func TestBaseURLOnlyTrustsMojangServiceHosts(t *testing.T) {
	disc := func(uri string) *service.Discovery {
		return &service.Discovery{ServiceEnvironments: map[string]map[string]json.RawMessage{
			"store": {"prod": json.RawMessage(`{"serviceUri":"` + uri + `"}`)},
		}}
	}
	if got := baseURL(disc("https://store.example.minecraft-services.net")); got.Host != "store.example.minecraft-services.net" {
		t.Fatalf("trusted host = %v", got)
	}
	for _, bad := range []string{"https://evil.example.test", "http://store.minecraft-services.net", "https://x@store.minecraft-services.net", "::"} {
		if got := baseURL(disc(bad)); got.String() != defaultBaseURI {
			t.Fatalf("%q selected %v", bad, got)
		}
	}
	if got := baseURL(nil); got.String() != defaultBaseURI {
		t.Fatalf("nil discovery = %v", got)
	}
}

func TestASuccessfulPurchaseRefreshesTheInventoryOnce(t *testing.T) {
	server := &purchaseServer{status: 200, body: `{"result":{}}`}
	client := newTestClient(t, server.handler(t), nil)
	if _, err := client.Purchase(context.Background(), request("purchase-0000000010", "offer-10")); err != nil {
		t.Fatal(err)
	}
	if server.refreshes.Load() != 1 {
		t.Fatalf("refreshes = %d", server.refreshes.Load())
	}
	refused := &purchaseServer{status: 422, body: `{}`}
	other := newTestClient(t, refused.handler(t), nil)
	if _, err := other.Purchase(context.Background(), request("purchase-0000000011", "offer-11")); err != nil || refused.refreshes.Load() != 0 {
		t.Fatalf("refused purchase refreshed %d times, err=%v", refused.refreshes.Load(), err)
	}
}

func TestMoreOffersPostsTheTokenAndMarksOwnership(t *testing.T) {
	var body layoutMoreBody
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case pathRowItems:
			if err := json.NewDecoder(r.Body).Decode(&body); err != nil || r.Method != http.MethodPost {
				t.Errorf("row request: %v %s", err, r.Method)
			}
			_, _ = io.WriteString(w, `{"result":{"continuationToken":"t2","items":[{"id":"AAAAAAAA-0000-0000-0000-000000000001","title":"Alpha"},{"id":"zzz","title":"Zed"}]}}`)
		default:
			w.Header().Set("InventoryETag", "etag-5")
			_, _ = io.WriteString(w, inventoryFixture)
		}
	}, nil)
	more, err := client.MoreOffers(context.Background(), "t1")
	if err != nil || body.ContinuationToken != "t1" || body.InventoryVersion != "etag-5" {
		t.Fatalf("body = %+v err=%v", body, err)
	}
	if len(more.Offers) != 2 || !more.Offers[0].Owned || more.Offers[1].Owned || more.Continuation != "t2" {
		t.Fatalf("more = %+v", more)
	}
	if _, err := client.MoreOffers(context.Background(), ""); !errors.Is(err, ErrInvalidRequest) {
		t.Fatalf("empty token err = %v", err)
	}
}

func TestRowsCarryTheirContinuationToken(t *testing.T) {
	page := parsePage("store", json.RawMessage(`{"rows":[{"title":"R","continuationToken":"next-1","offers":[{"id":"a","title":"A"}]},{"title":"S","offers":[{"id":"b","title":"B"}]}]}`))
	if len(page.Rows) != 2 || page.Rows[0].Continuation != "next-1" || page.Rows[1].Continuation != "" {
		t.Fatalf("rows = %+v", page.Rows)
	}
}

func TestEntitlementsRefreshAsksTheServiceFirst(t *testing.T) {
	var refreshes atomic.Int32
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == pathRefresh {
			refreshes.Add(1)
			_, _ = io.WriteString(w, `{"result":{"version":"v2"}}`)
			return
		}
		_, _ = io.WriteString(w, inventoryFixture)
	}, nil)
	if _, err := client.Entitlements(context.Background(), 0, 0, true); err != nil || refreshes.Load() != 1 {
		t.Fatalf("refreshes = %d err=%v", refreshes.Load(), err)
	}
	if _, err := client.Entitlements(context.Background(), 0, 0, false); err != nil || refreshes.Load() != 1 {
		t.Fatalf("a cached read refreshed: %d err=%v", refreshes.Load(), err)
	}
}

// A path or redirect that changes the origin must never carry the service token off-host.
func TestTokenNeverLeavesTheServiceOrigin(t *testing.T) {
	var leaked atomic.Int32
	other := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		leaked.Add(1)
	}))
	t.Cleanup(other.Close)
	otherURL, _ := url.Parse(other.URL)
	client := newTestClient(t, func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, other.URL+"/steal", http.StatusFound)
	}, nil)
	for _, path := range []string{"//" + otherURL.Host + "/steal", "https://" + otherURL.Host + "/steal", "http://" + otherURL.Host + "/steal", "//user@" + otherURL.Host} {
		if _, err := client.do(context.Background(), http.MethodGet, path, nil); !errors.Is(err, ErrInvalidRequest) {
			t.Errorf("%q err = %v", path, err)
		}
	}
	if _, err := client.do(context.Background(), http.MethodGet, pathBalances, nil); !errors.Is(err, ErrInvalidRequest) {
		t.Errorf("cross-origin redirect err = %v", err)
	}
	if leaked.Load() != 0 {
		t.Fatalf("other origin received %d requests", leaked.Load())
	}
}

// Price options the purchase flow cannot express are refused, never flattened into separate prices.
func TestOffersKeepOnlyWholeSingleCurrencyPrices(t *testing.T) {
	item := func(options ...playfabcatalog.Price) *playfabcatalog.Item {
		return &playfabcatalog.Item{ID: "offer-1", Title: map[string]string{"NEUTRAL": "Pack"}, PriceOptions: options}
	}
	mc := func(value int) playfabcatalog.PriceAmount {
		return playfabcatalog.PriceAmount{Value: value, ItemID: "mc"}
	}
	combined := playfabcatalog.Price{Amounts: []playfabcatalog.PriceAmount{mc(100), {Value: 5, ItemID: "tokens"}}}
	timed := playfabcatalog.Price{Amounts: []playfabcatalog.PriceAmount{mc(100)}, UnitDurationInSeconds: 86400}
	bulk := playfabcatalog.Price{Amounts: []playfabcatalog.PriceAmount{mc(100)}, UnitAmount: 5}
	single := playfabcatalog.Price{Amounts: []playfabcatalog.PriceAmount{mc(320)}}

	offer, ok := offerFromItem(item(combined, single, timed))
	if !ok || len(offer.Prices) != 1 || offer.Prices[0] != (Price{Currency: "mc", Amount: 320}) {
		t.Fatalf("offer = %+v ok = %v", offer, ok)
	}
	for name, option := range map[string]playfabcatalog.Price{"combined": combined, "timed": timed, "bulk": bulk} {
		if _, ok := offerFromItem(item(option)); ok {
			t.Fatalf("%s-only offer was listed", name)
		}
	}
	if offer, ok := offerFromItem(item()); !ok || len(offer.Prices) != 0 {
		t.Fatalf("an unpriced offer = %+v ok = %v", offer, ok)
	}
}
