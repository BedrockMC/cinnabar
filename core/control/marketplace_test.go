package control

import (
	"context"
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/hashimthearab/rust-mcbe/core/store"
)

type stubMarket struct {
	page     store.Page
	purchase store.PurchaseRequest
	result   store.PurchaseResult
	err      error
	homeArg  string
	calls    int
}

func (m *stubMarket) Home(_ context.Context, page string) (store.Page, error) {
	m.homeArg = page
	return m.page, m.err
}
func (m *stubMarket) Search(context.Context, store.SearchQuery) (store.SearchResults, error) {
	return store.SearchResults{}, m.err
}
func (m *stubMarket) Offer(context.Context, string) (store.OfferDetail, error) {
	return store.OfferDetail{}, m.err
}
func (m *stubMarket) Balances(context.Context) ([]store.Balance, error) { return nil, m.err }
func (m *stubMarket) Entitlements(context.Context, int, int) (store.Entitlements, error) {
	return store.Entitlements{Owned: []string{"a"}, Total: 1}, m.err
}
func (m *stubMarket) Image(context.Context, string) (store.Image, error) {
	return store.Image{Path: "/tmp/x.png", ContentType: "image/png"}, m.err
}
func (m *stubMarket) Purchase(_ context.Context, r store.PurchaseRequest) (store.PurchaseResult, error) {
	m.calls++
	m.purchase = r
	return m.result, m.err
}

func startMarket(t *testing.T, m Marketplace) string {
	t.Helper()
	dir := t.TempDir()
	server, err := Start(dir, NewStore())
	if err != nil {
		t.Fatal(err)
	}
	if m != nil {
		server.SetMarketplace(m)
	}
	t.Cleanup(func() { _ = server.Close() })
	return dir
}

const validPurchase = `{"purchase_id":"0123456789abcdef","offer_id":"offer-1","currency":"mc","amount":"320","confirmed":true}`

func TestStoreMethodsWithoutMarketplaceAreRejected(t *testing.T) {
	dir := startMarket(t, nil)
	if reply := rpc(t, dir, methodStoreBalance, ""); reply.Error == nil || reply.Error.Code != codeServicesDisabled {
		t.Fatalf("error = %+v", reply.Error)
	}
}

func TestStoreHomeDefaultsPageAndEncodesEmptyRows(t *testing.T) {
	m := &stubMarket{page: store.Page{ID: "store"}}
	dir := startMarket(t, m)
	raw := string(call(t, dir, methodStoreHome, ""))
	if m.homeArg != "store" || !strings.Contains(raw, `"rows":[]`) {
		t.Fatalf("page=%q response=%s", m.homeArg, raw)
	}
	if reply := rpc(t, dir, methodStoreHome, `{"page":"marketplacepass"}`); reply.Error != nil || m.homeArg != "marketplacepass" {
		t.Fatalf("named page: %+v arg=%q", reply.Error, m.homeArg)
	}
}

func TestStorePurchaseRequiresExplicitConfirmation(t *testing.T) {
	m := &stubMarket{result: store.PurchaseResult{Status: store.PurchaseOK, CorrelationID: "c"}}
	dir := startMarket(t, m)
	for _, params := range []string{
		``, `{}`,
		strings.Replace(validPurchase, `"confirmed":true`, `"confirmed":false`, 1),
		strings.Replace(validPurchase, `,"confirmed":true`, ``, 1),
		strings.Replace(validPurchase, `"amount":"320"`, `"amount":320`, 1),
		strings.TrimSuffix(validPurchase, "}") + `,"extra":1}`,
	} {
		if reply := rpc(t, dir, methodStorePurchase, params); reply.Error == nil || reply.Error.Code != -32602 {
			t.Fatalf("params %q error = %+v", params, reply.Error)
		}
	}
	if m.calls != 0 {
		t.Fatalf("unconfirmed purchase reached the service %d times", m.calls)
	}
	var result storePurchaseResultV1
	reply := rpc(t, dir, methodStorePurchase, validPurchase)
	if reply.Error != nil || json.Unmarshal(reply.Result, &result) != nil || result.Status != store.PurchaseOK ||
		result.SchemaVersion != 1 || m.purchase.Amount != "320" || !m.purchase.Confirmed {
		t.Fatalf("purchase = %+v / %+v", result, reply.Error)
	}
}

func TestStoreErrorsAreSanitized(t *testing.T) {
	for _, test := range []struct {
		err  error
		code int
	}{
		{ErrSignedOut, codeSignedOut},
		{store.ErrInvalidRequest, -32602},
		{store.ErrPurchaseBusy, codePurchaseBusy},
		{store.ErrPurchaseReused, codePurchaseReused},
		{store.ErrUnknownPage, codeStoreNotFound},
		{errors.New(`Post https://x/y?token=SECRET: dial tcp`), codeServiceFailed},
	} {
		dir := startMarket(t, &stubMarket{err: test.err})
		reply := rpc(t, dir, methodStorePurchase, validPurchase)
		if reply.Error == nil || reply.Error.Code != test.code || strings.Contains(reply.Error.Message, "SECRET") {
			t.Fatalf("error for %v = %+v", test.err, reply.Error)
		}
	}
}

func TestFitPageKeepsResponsesInsideOneFrame(t *testing.T) {
	page := store.Page{ID: "store"}
	for r := 0; r < 40; r++ {
		row := store.Row{ID: "r"}
		for o := 0; o < 40; o++ {
			row.Offers = append(row.Offers, store.Offer{ID: "id", Title: strings.Repeat("t", 250), Creator: strings.Repeat("c", 250)})
		}
		page.Rows = append(page.Rows, row)
	}
	fitted := fitPage(page)
	if !fitted.Truncated || encodedLen(fitted) > frameBudget || len(fitted.Rows) == 0 {
		t.Fatalf("truncated=%v len=%d rows=%d", fitted.Truncated, encodedLen(fitted), len(fitted.Rows))
	}
}
