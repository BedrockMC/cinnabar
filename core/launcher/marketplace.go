package launcher

import (
	"context"

	"github.com/hashimthearab/rust-mcbe/core/control"
	"github.com/hashimthearab/rust-mcbe/core/store"
)

// Storefront is the store service the launcher gates on the signed-in account.
type Storefront interface {
	Home(ctx context.Context, page string) (store.Page, error)
	Search(ctx context.Context, q store.SearchQuery) (store.SearchResults, error)
	Offer(ctx context.Context, id string) (store.OfferDetail, error)
	Balances(ctx context.Context) ([]store.Balance, error)
	Entitlements(ctx context.Context, offset, limit int) (store.Entitlements, error)
	Purchase(ctx context.Context, r store.PurchaseRequest) (store.PurchaseResult, error)
}

type marketplace struct {
	svc   *Service
	front Storefront
}

// Marketplace returns the store_* backend; every call fails with control.ErrSignedOut once the account is
// signed out. A nil front opens a real store session with the account's token source on first use.
func (s *Service) Marketplace(front Storefront) control.Marketplace {
	if front == nil {
		front = store.NewSession(s.cfg.TokenSource)
	}
	return marketplace{svc: s, front: front}
}

func (m marketplace) Home(ctx context.Context, page string) (store.Page, error) {
	if _, err := m.svc.source(); err != nil {
		return store.Page{}, err
	}
	return m.front.Home(ctx, page)
}

func (m marketplace) Search(ctx context.Context, q store.SearchQuery) (store.SearchResults, error) {
	if _, err := m.svc.source(); err != nil {
		return store.SearchResults{}, err
	}
	return m.front.Search(ctx, q)
}

func (m marketplace) Offer(ctx context.Context, id string) (store.OfferDetail, error) {
	if _, err := m.svc.source(); err != nil {
		return store.OfferDetail{}, err
	}
	return m.front.Offer(ctx, id)
}

func (m marketplace) Balances(ctx context.Context) ([]store.Balance, error) {
	if _, err := m.svc.source(); err != nil {
		return nil, err
	}
	return m.front.Balances(ctx)
}

func (m marketplace) Entitlements(ctx context.Context, offset, limit int) (store.Entitlements, error) {
	if _, err := m.svc.source(); err != nil {
		return store.Entitlements{}, err
	}
	return m.front.Entitlements(ctx, offset, limit)
}

func (m marketplace) Purchase(ctx context.Context, r store.PurchaseRequest) (store.PurchaseResult, error) {
	if _, err := m.svc.source(); err != nil {
		return store.PurchaseResult{}, err
	}
	return m.front.Purchase(ctx, r)
}
