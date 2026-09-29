package store

import (
	"encoding/json"
	"strings"
)

const (
	maxPageRows      = 60
	maxPageOffers    = 120
	maxRowOffers     = 40
	maxStringLen     = 256
	maxWalkDepth     = 8
	maxTagsPerOffer  = 8
	maxPricesPerItem = 4
)

// Members tried, in order, for each offer field of a layout page; the page schema is server-defined so
// unknown shapes are skipped rather than failing the page.
var (
	rowListKeys   = []string{"rows", "sections", "components", "children"}
	offerListKeys = []string{"offers", "items", "content", "products"}
	idKeys        = []string{"id", "offerId", "itemId", "productId"}
	titleKeys     = []string{"title", "name", "displayName", "header", "text"}
	creatorKeys   = []string{"creator", "creatorName", "publisher"}
	imageKeys     = []string{"thumbnail", "thumbnailUrl", "image", "imageUrl", "icon"}
	priceKeys     = []string{"price", "prices", "minecoinPrice", "cost"}
)

// parsePage reduces a layout result to titled rows of offers, bounded so it fits a control frame.
func parsePage(id string, result json.RawMessage) Page {
	page := Page{ID: id, Rows: []Row{}}
	var top any
	if json.Unmarshal(result, &top) != nil {
		return page
	}
	if obj, ok := top.(map[string]any); ok {
		if inner, ok := obj["page"]; ok {
			top = inner
		}
	}
	total := 0
	var walk func(node any, depth int)
	walk = func(node any, depth int) {
		if depth > maxWalkDepth || page.Truncated {
			return
		}
		obj, ok := node.(map[string]any)
		if !ok {
			if arr, ok := node.([]any); ok {
				for _, child := range arr {
					walk(child, depth+1)
				}
			}
			return
		}
		if offers := offersOf(obj); len(offers) > 0 {
			if len(page.Rows) >= maxPageRows || total+len(offers) > maxPageOffers {
				page.Truncated = true
				return
			}
			total += len(offers)
			page.Rows = append(page.Rows, Row{
				ID: firstString(obj, "id", "rowId", "name"), Title: firstText(obj, titleKeys...),
				Kind: firstString(obj, "type", "kind", "layout"), Offers: offers,
				Continuation: continuationOf(obj),
			})
			return
		}
		for _, key := range rowListKeys {
			if arr, ok := obj[key].([]any); ok {
				for _, child := range arr {
					walk(child, depth+1)
				}
			}
		}
	}
	walk(top, 0)
	return page
}

// offersOf returns the offers listed directly under a row object.
func offersOf(row map[string]any) []Offer {
	for _, key := range offerListKeys {
		arr, ok := row[key].([]any)
		if !ok {
			continue
		}
		var offers []Offer
		for _, entry := range arr {
			obj, ok := entry.(map[string]any)
			if !ok {
				continue
			}
			if offer, ok := offerFromLayout(obj, firstString(row, "storeId")); ok {
				offers = append(offers, offer)
				if len(offers) >= maxRowOffers {
					break
				}
			}
		}
		if len(offers) > 0 {
			return offers
		}
	}
	return nil
}

func offerFromLayout(obj map[string]any, rowStore string) (Offer, bool) {
	id := firstString(obj, idKeys...)
	title := firstText(obj, titleKeys...)
	if id == "" || !ValidOfferID(id) || title == "" {
		return Offer{}, false
	}
	offer := Offer{
		ID: id, Title: title, Creator: firstText(obj, creatorKeys...),
		ContentType: firstString(obj, "contentType", "type"),
		StoreID:     firstString(obj, "storeId"),
	}
	if offer.StoreID == "" {
		offer.StoreID = rowStore
	}
	for _, key := range imageKeys {
		if url := imageURL(obj[key]); url != "" {
			offer.ThumbnailURL = url
			break
		}
	}
	for _, key := range priceKeys {
		if prices := pricesOf(obj[key]); len(prices) > 0 {
			offer.Prices = prices
			break
		}
	}
	if tags, ok := obj["tags"].([]any); ok {
		for _, tag := range tags {
			if s, ok := tag.(string); ok && s != "" && len(offer.Tags) < maxTagsPerOffer {
				offer.Tags = append(offer.Tags, clip(s))
			}
		}
	}
	return offer, true
}

// pricesOf reads a price given as a number, an {currency, amount} object or a list of either.
func pricesOf(v any) []Price {
	var out []Price
	add := func(currency string, amount any) {
		var n flexInt
		raw, err := json.Marshal(amount)
		if err != nil || n.UnmarshalJSON(raw) != nil || n < 0 || len(out) >= maxPricesPerItem {
			return
		}
		out = append(out, Price{Currency: currency, Amount: int64(n)})
	}
	switch p := v.(type) {
	case float64, string:
		add("", p)
	case map[string]any:
		add(firstString(p, "currency", "type", "currencyId", "itemId"), firstAny(p, "amount", "value", "price"))
	case []any:
		for _, e := range p {
			out = append(out, pricesOf(e)...)
		}
		if len(out) > maxPricesPerItem {
			out = out[:maxPricesPerItem]
		}
	}
	return out
}

func firstAny(obj map[string]any, keys ...string) any {
	for _, key := range keys {
		if v, ok := obj[key]; ok {
			return v
		}
	}
	return nil
}

// imageURL accepts an https URL string, an {url} object or a list whose first entry is one.
func imageURL(v any) string {
	switch i := v.(type) {
	case string:
		if strings.HasPrefix(i, "https://") && len(i) <= 1024 {
			return i
		}
	case map[string]any:
		return imageURL(firstAny(i, "url", "Url", "href"))
	case []any:
		if len(i) > 0 {
			return imageURL(i[0])
		}
	}
	return ""
}

func firstString(obj map[string]any, keys ...string) string {
	for _, key := range keys {
		if s, ok := obj[key].(string); ok && s != "" {
			return clip(s)
		}
	}
	return ""
}

// firstText reads a string member, or the neutral/first value of a localized object.
func firstText(obj map[string]any, keys ...string) string {
	for _, key := range keys {
		switch t := obj[key].(type) {
		case string:
			if t != "" {
				return clip(t)
			}
		case map[string]any:
			if s := localized(t); s != "" {
				return s
			}
		}
	}
	return ""
}

func localized(m map[string]any) string {
	var fallback, fallbackKey string
	for key, v := range m {
		s, ok := v.(string)
		if !ok || s == "" {
			continue
		}
		if strings.EqualFold(key, "neutral") || strings.EqualFold(key, "en-us") {
			return clip(s)
		}
		if fallbackKey == "" || key < fallbackKey {
			fallback, fallbackKey = clip(s), key
		}
	}
	return fallback
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

// continuationOf reads a row's continuation token, dropping one too long to send back.
func continuationOf(obj map[string]any) string {
	token := firstString(obj, "continuationToken", "continuation")
	if !ValidContinuation(token) {
		return ""
	}
	return token
}

// parseRowMore reduces a row-continuation result to its offers and next token; the offers may sit
// under any row-shaped member of the result.
func parseRowMore(result json.RawMessage) RowMore {
	more := RowMore{Offers: []Offer{}}
	var top any
	if json.Unmarshal(result, &top) != nil {
		return more
	}
	obj, ok := top.(map[string]any)
	if !ok {
		return more
	}
	more.Continuation = continuationOf(obj)
	page := parsePage("", result)
	for _, row := range page.Rows {
		for _, offer := range row.Offers {
			if len(more.Offers) < maxRowOffers {
				more.Offers = append(more.Offers, offer)
			}
		}
		if more.Continuation == "" {
			more.Continuation = row.Continuation
		}
	}
	return more
}
