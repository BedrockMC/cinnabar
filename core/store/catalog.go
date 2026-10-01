package store

import (
	"context"
	"encoding/json"
	"errors"
	"strings"

	playfabcatalog "github.com/df-mc/go-playfab/v2/catalog"
	"golang.org/x/text/language"
)

const (
	defaultSearchCount = 24
	maxSearchOffers    = 50
)

// Search runs a catalog search against PlayFab as the account and marks owned offers.
func (c *Client) Search(ctx context.Context, q SearchQuery) (SearchResults, error) {
	if err := q.Validate(); err != nil {
		return SearchResults{}, err
	}
	if q.Count == 0 {
		q.Count = defaultSearchCount
	}
	result, err := c.cfg.Catalog.SearchItems(ctx, playfabcatalog.SearchFilter{
		Count: q.Count, ContinuationToken: q.Continuation, Filter: q.Filter, OrderBy: q.OrderBy,
		Term: q.Term, Language: language.AmericanEnglish,
	})
	if err != nil {
		return SearchResults{}, err
	}
	if result == nil {
		return SearchResults{}, errors.New("store: empty search result")
	}
	out := SearchResults{Offers: make([]Offer, 0, len(result.Items)), Continuation: result.ContinuationToken}
	for i := range result.Items {
		if len(out.Offers) >= maxSearchOffers {
			out.Truncated = true
			break
		}
		if offer, ok := offerFromItem(&result.Items[i]); ok {
			out.Offers = append(out.Offers, offer)
		}
	}
	c.markOwned(ctx, out.Offers)
	return out, nil
}

// Offer returns the detail of one offer from the catalog.
func (c *Client) Offer(ctx context.Context, id string) (OfferDetail, error) {
	if !ValidOfferID(id) {
		return OfferDetail{}, ErrInvalidRequest
	}
	item, err := c.cfg.Catalog.ItemByID(ctx, id)
	if err != nil {
		return OfferDetail{}, err
	}
	offer, ok := offerFromItem(item)
	if !ok {
		return OfferDetail{}, errNoOffer
	}
	detail := OfferDetail{Offer: offer, DisplayVersion: clip(item.DisplayVersion), Description: firstLocalized(item.Description)}
	if len(detail.Description) > 8192 {
		detail.Description = detail.Description[:8192]
	}
	for _, img := range item.Images {
		if strings.EqualFold(img.Type, playfabcatalog.ImageTypeScreenshot) && strings.HasPrefix(img.URL, "https://") && len(detail.ScreenshotURLs) < 12 {
			detail.ScreenshotURLs = append(detail.ScreenshotURLs, img.URL)
		}
	}
	for _, p := range item.Platforms {
		if len(detail.Platforms) < 16 {
			detail.Platforms = append(detail.Platforms, clip(p))
		}
	}
	one := []Offer{detail.Offer}
	c.markOwned(ctx, one)
	detail.Owned = one[0].Owned
	return detail, nil
}

func firstLocalized(d playfabcatalog.Dictionary[string]) string {
	if v, ok := d.Lookup("en-US"); ok && v != "" {
		return v
	}
	return d.Neutral()
}

// offerFromItem maps a PlayFab catalog item; an item without an id or title is skipped.
func offerFromItem(item *playfabcatalog.Item) (Offer, bool) {
	if item == nil || item.ID == "" || item.Hidden {
		return Offer{}, false
	}
	title := clip(firstLocalized(item.Title))
	if title == "" {
		return Offer{}, false
	}
	offer := Offer{ID: item.ID, Title: title, ContentType: clip(item.ContentType)}
	var props struct {
		Creator string `json:"creatorName"`
	}
	if len(item.DisplayProperties) > 0 {
		_ = json.Unmarshal(item.DisplayProperties, &props)
	}
	offer.Creator = clip(props.Creator)
	offer.ThumbnailURL = thumbnailOf(item.Images)
	for _, option := range item.PriceOptions {
		if price, ok := singlePrice(option); ok && len(offer.Prices) < maxPricesPerItem {
			offer.Prices = append(offer.Prices, price)
		}
	}
	if len(item.PriceOptions) > 0 && len(offer.Prices) == 0 {
		return Offer{}, false // only price forms the store cannot quote or buy
	}
	if item.Rating.TotalCount > 0 {
		offer.Rating = &Rating{Average: float64(item.Rating.Average), Count: item.Rating.TotalCount}
	}
	for _, tag := range item.Tags {
		if len(offer.Tags) < maxTagsPerOffer {
			offer.Tags = append(offer.Tags, clip(tag))
		}
	}
	return offer, true
}

// singlePrice maps a price option the store can quote and buy: one currency amount for one unit
// with no duration. Options needing several currencies together, several units or a duration are
// refused rather than split into prices the purchase flow would misread.
func singlePrice(option playfabcatalog.Price) (Price, bool) {
	if len(option.Amounts) != 1 || option.UnitAmount > 1 || option.UnitDurationInSeconds != 0 {
		return Price{}, false
	}
	amount := option.Amounts[0]
	if amount.Value < 0 || amount.ItemID == "" {
		return Price{}, false
	}
	return Price{Currency: amount.ItemID, Amount: int64(amount.Value)}, true
}

func thumbnailOf(images []playfabcatalog.Image) string {
	var fallback string
	for _, img := range images {
		if !strings.HasPrefix(img.URL, "https://") || len(img.URL) > 1024 {
			continue
		}
		if strings.EqualFold(img.Type, playfabcatalog.ImageTypeThumbnail) || strings.EqualFold(img.Tag, "thumbnail") {
			return img.URL
		}
		if fallback == "" {
			fallback = img.URL
		}
	}
	return fallback
}
