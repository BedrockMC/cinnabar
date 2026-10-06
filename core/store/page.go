package store

import (
	"context"
	"regexp"
	"strings"
	"sync"

	"github.com/sandertv/gophertunnel/minecraft/service/marketplace"
	"golang.org/x/text/language"
)

const (
	maxPageRows      = 60
	maxRowOffers     = 40
	maxStringLen     = 256
	maxTagsPerOffer  = 8
	maxPricesPerItem = 4
	rowSearchWorkers = 6
)

var pagePattern = regexp.MustCompile(`^[A-Za-z0-9_.-]{1,64}$`)

// Home loads a known store page by its session-config name. Curated rows carry their offers inline;
// a query row is filled by running its first query the catalog search can express.
func (c *Client) Home(ctx context.Context, name string) (Page, error) {
	if !pagePattern.MatchString(name) {
		return Page{}, ErrInvalidRequest
	}
	cfg, err := c.sessionConfig(ctx)
	if err != nil {
		return Page{}, err
	}
	id, err := cfg.PageID(name)
	if err != nil {
		return Page{}, err
	}
	inv, err := c.loadInventory(ctx, false)
	if err != nil {
		return Page{}, err
	}
	c.mu.Lock()
	state := marketplace.PageRequest{Entitlements: inv.ids, InventoryVersion: c.etag, ListVersion: c.lists}
	c.mu.Unlock()
	if state.Entitlements == nil {
		state.Entitlements = []string{}
	}
	layout, err := c.cfg.Market.Page(ctx, marketplace.PageByID, id, state)
	if err != nil {
		return Page{}, err
	}
	c.mu.Lock()
	if layout.HeaderInventoryETag != "" {
		c.etag = layout.HeaderInventoryETag
	}
	if layout.HeaderListsVersion != "" {
		c.lists = layout.HeaderListsVersion
	}
	version := c.etag
	c.mu.Unlock()

	page := Page{ID: name, Rows: []Row{}, InventoryVersion: version}
	var rows []Row // page order; a query row stays empty until filled
	var pending []pendingRow
	for _, section := range layout.Layout {
		for i := range section.Rows {
			row := &section.Rows[i]
			if len(rows) >= maxPageRows {
				page.Truncated = true
				break
			}
			if list := row.ItemList(); list != nil && len(list.Items) > 0 {
				rows = append(rows, c.curatedRow(row, list))
			} else if query, ok := row.SearchQuery(); ok {
				pending = append(pending, pendingRow{slot: len(rows), row: row, query: query})
				rows = append(rows, Row{})
			}
		}
	}
	filled, err := c.fillRows(ctx, pending)
	if err != nil && len(pending) == len(rows) {
		return Page{}, err // every row was a query row and every search failed
	}
	if err == nil {
		for i, p := range pending {
			rows[p.slot] = filled[i]
		}
	}
	for _, row := range rows {
		if len(row.Offers) > 0 {
			page.Rows = append(page.Rows, row)
		}
	}
	return page, nil
}

type pendingRow struct {
	slot  int
	row   *marketplace.Row
	query marketplace.Query
}

// curatedRow maps a row whose item list carries its offers inline.
func (c *Client) curatedRow(row *marketplace.Row, list *marketplace.Component) Row {
	out := newRow(row)
	for i := range list.Items {
		if offer, ok := offerFromMarketItem(&list.Items[i]); ok && len(out.Offers) < maxRowOffers {
			offer.Owned = c.owned(offer.ID)
			out.Offers = append(out.Offers, offer)
		}
	}
	return out
}

func newRow(row *marketplace.Row) Row {
	out := Row{ID: clip(row.TelemetryID), Title: clip(row.Title()), Offers: []Offer{}}
	if len(row.Components) > 0 {
		out.Kind = clip(row.Components[0].Type)
	}
	return out
}

// fillRows runs each row's query; a failed search drops its row, and the page fails only when every
// search failed.
func (c *Client) fillRows(ctx context.Context, pending []pendingRow) ([]Row, error) {
	rows := make([]Row, len(pending))
	errs := make([]error, len(pending))
	jobs := make(chan int)
	var wait sync.WaitGroup
	for range min(rowSearchWorkers, len(pending)) {
		wait.Add(1)
		go func() {
			defer wait.Done()
			for index := range jobs {
				rows[index], errs[index] = c.fillRow(ctx, pending[index])
			}
		}()
	}
	for index := range pending {
		jobs <- index
	}
	close(jobs)
	wait.Wait()
	var firstErr error
	failed := 0
	for _, err := range errs {
		if err != nil {
			failed++
			if firstErr == nil {
				firstErr = err
			}
		}
	}
	if failed > 0 && failed == len(pending) {
		return nil, firstErr
	}
	return rows, nil
}

func (c *Client) fillRow(ctx context.Context, p pendingRow) (Row, error) {
	filter, _ := p.query.SearchFilter()
	filter.Language = language.AmericanEnglish
	result, err := c.cfg.Catalog.SearchItems(ctx, filter)
	if err != nil {
		return Row{}, err
	}
	row := newRow(p.row)
	for i := range result.Items {
		if offer, ok := offerFromItem(&result.Items[i]); ok && len(row.Offers) < maxRowOffers {
			offer.Owned = c.owned(offer.ID)
			row.Offers = append(row.Offers, offer)
		}
	}
	return row, nil
}

// offerFromMarketItem maps a store-service item, inline on a page or from a row continuation; an
// item without an id or title is skipped.
func offerFromMarketItem(item *marketplace.Item) (Offer, bool) {
	id := strings.ToLower(item.ID)
	title := clip(item.Title.Neutral())
	if !ValidOfferID(id) || title == "" {
		return Offer{}, false
	}
	offer := Offer{
		ID: id, Title: title, Creator: clip(item.CreatorName),
		ContentType: clip(item.ContentType), StoreID: clip(item.StoreID),
	}
	images := item.Images
	if item.Thumbnail != nil {
		images = append([]marketplace.Image{*item.Thumbnail}, images...)
	}
	for _, image := range images {
		if strings.EqualFold(image.Type, "Thumbnail") && strings.HasPrefix(image.URL, "https://") && len(image.URL) <= 1024 {
			offer.ThumbnailURL = image.URL
			break
		}
	}
	if item.Rating != nil && item.Rating.TotalCount > 0 {
		offer.Rating = &Rating{Average: item.Rating.Average, Count: item.Rating.TotalCount}
	}
	if price := item.Price; price != nil {
		amount := int64(price.ListPrice)
		if price.Sale != nil && price.Sale.SalePrice > 0 {
			amount = price.Sale.SalePrice
		}
		offer.Prices = []Price{{Currency: price.CurrencyID, Amount: amount}}
	}
	for _, tag := range item.Tags {
		if len(offer.Tags) < maxTagsPerOffer {
			offer.Tags = append(offer.Tags, clip(tag))
		}
	}
	return offer, true
}

func clip(s string) string {
	if len(s) <= maxStringLen {
		return s
	}
	cut := maxStringLen
	for cut > 0 && s[cut]&0xC0 == 0x80 { // do not split a UTF-8 sequence
		cut--
	}
	return s[:cut]
}
