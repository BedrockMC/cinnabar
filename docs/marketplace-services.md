# Marketplace services

Behaviour reference for the 26.30 client's store. Control surface: `docs/control-channel.md` (`store_*.v1`); Go: `core/store`.
Status per row: **ref** = read from the 26.30 client reconstruction, **doc** = public PlayFab documentation, **guess** = inferred,
needs one live capture against a sandbox account before it is relied on.

## Auth chain

1. Microsoft account (device code) -> XSTS via `go-xsapi`.
2. PlayFab `Client/LoginWithXbox` on the title id from discovery `auth.prod.playFabTitleId` (`20CA2` on retail) -> session ticket + entity token.
3. Mojang authorization service `POST {auth.serviceUri}/api/v1.0/session/start` with the ticket -> **MCToken** (`Authorization: MCToken ...`). ref
4. Discovery: `GET client.discovery.minecraft-services.net/api/v1.0/discovery/MinecraftPE/builds/{version}` names every service base URI.

| Token | Used for |
| --- | --- |
| PlayFab entity token (`X-EntityToken`, `master_player_account`) | PlayFab Catalog calls |
| MCToken (`Authorization`) + `Session-Id` header | every store-service call below |

The store base URI is a discovery service environment whose name is not recoverable from the reconstruction (candidates tried: `store`,
`marketplace`, `mktpl`; fallback `store.mktpl.minecraft-services.net`). guess. Discovery URIs outside `*.minecraft-services.net` over
https are ignored so the MCToken never leaves Mojang's domain.

## Service calls

Store-service paths are relative to the store base URI. Every answer is wrapped as `{"result": ...}`; errors are
`{namespace, code, message, customData}`.

| Function | Call | Notes |
| --- | --- | --- |
| Session config | `GET /api/v1.0/session/config` | ref. Members: `knownPages` (name -> layout path), `latestTextureVersion`, filter collections, dressing-room filters, per-category terms, `platformSkus`, `feedbackCharacterLimit`, `badgePromoCountdownWindow` |
| Home / rows / upsell pages | `POST {knownPages[name]}` body `{entitlements: [uuid], inventoryVersion, listVersion}` | ref body; the path is server-supplied and the client appends a per-request-type suffix table that is not recoverable (guess: `knownPages` value is used as-is). Answer headers `InventoryETag`, `X-UserLists-Version`; body `{page, inventoryVersion, sidebarLayoutType, userListsVersion}` with `page.layout.sections[].rows` (guess for inner names) |
| Row continuation | `POST /api/v2.0/layout/items` body `{continuationToken, inventoryVersion}` | ref path and body; `store_row_more.v1`; the result shape is a guess (parsed like a page) |
| Search (marketplace) | `POST` search-layout body: filters (`price`, `rating`, `...FilterTag` tag filters, range `{type,value,high,low}`), `installedPackIds`, `filterCurrentRealmsPlus`, `filterPastRealmsPlus` | ref member names only; core uses the catalog search below |
| Catalog search | PlayFab `POST /Catalog/Search` (`/Catalog/SearchStores` for store scope) | ref path; body is the PlayFab economy search filter. Core uses the documented `Catalog/SearchItems` through `go-playfab` (doc) |
| Offer detail | PlayFab `/Catalog/GetPublishedItem`; core uses documented `Catalog/GetItem` via `go-playfab` | ref / doc |
| Ratings | read: PlayFab `/Catalog/GetItemReviewSummary`, `/Catalog/GetMyReview`; write: PlayFab `/Catalog/CreateOrUpdateReview` or store `POST /api/v1.0/catalog/reviewitem` body `{ItemId, Rating}` | ref; core returns `Item.Rating` only, no rating writes |
| Minecoin balance | `GET /api/v1.0/currencies/virtual/balances` | ref. `result.virtualCurrencyBalances: [{type, amount}]`; `type` also takes a PlayStation token value |
| Entitlements | `GET /api/v1.0/player/inventory?includeReceipt=true` (the query is 20 characters in the client; the flag name is a guess) | ref path; result shape guess, parsed leniently for id members |
| Inventory refresh | `POST /api/v1.0/inventory/refresh` (empty object body) | ref path and body; method and answer `{result: {version}}`; the core calls it after every successful purchase and on `store_entitlements.v1 {refresh: true}` |
| **Minecoin purchase** | `POST /api/v1.0/transaction/virtual` | ref, below |
| Real-money top-up redeem | `POST /api/v1.0/transaction/redeem` | ref, below; not implemented |
| Feedback / safety | `/api/v1.0/feedback`, `/api/v1.0/messages/*` | out of scope |

### Purchase (ref)

Body:

```json
{"VirtualCurrency": {"Type": "<currency>", "Amount": "<decimal string>"},
 "OfferId": "<id>", "StoreId": "<id>", "UnitDurationInSeconds": 0,
 "CustomTags": {"ClientId": "", "DeviceSessionId": "", "CorrelationId": "", "TitleId": "20CA2",
                "BuildPlat": 7, "editionType": "", "Seq": 1, "DnAPlat": "", "Xuid": ""}}
```

`UnitDurationInSeconds` is present only for subscription offers. Response: header `InventoryEtag`; on non-2xx the body is
`{"code": "PlayFabError", "customData": {"marketplaceErrorCode": "<int>"}}` (missing or malformed -> 1502).
HTTP 2xx = purchased, 422 and 412 each get their own client outcome, anything else is a generic failure. The 422 outcome is shown with
`store.popup.purchasePriceMismatch.msg` (inferred, guess); the purchase-failed dialog is `store.popup.purchaseFailed.*`; a service error
dialog shows the marketplace error code and correlation id (`store.csb.purchaseErrorDialog.*`).

Unverified vocabularies (a wrong value fails the purchase, it cannot make one free): `Type` (the balance `type` vs the catalog price item id),
`StoreId` origin, `editionType`, `DnAPlat`, `Session-Id`. The core sends `BuildPlat` 7 and `Windows10` to match the device the auth
token claims.

### Real-money top-up (documented, not implemented)

Minecoins are bought in the platform store (Microsoft Store here), not through Mojang. The client then redeems the store receipt:
`POST /api/v1.0/transaction/redeem` with the common `CustomTags` block plus `platformPurchaseId`, the platform receipt fields,
`passSubscription`, `sku` and a production/sandbox flag. The coin-bundle screens (`coin_purchase_screen.json`, `MinecoinCatalogModel`)
list the bundles and open the platform store. Cinnabar has no platform-store integration: the insufficient-funds dialog
(`store.popup.purchaseFailedInsufficientFunds.*`) is shown, and its "Get Minecoins" button is where a top-up would attach.

## Owned content

Ownership comes from the inventory call; layout requests send every owned id so the service marks rows. Offer `owned` in the control
results is a lookup in that list (refreshed after a purchase). Downloading or decrypting owned packs for worlds is out of scope.

## Risks

- Third-party clients spending real Minecoins with client-supplied telemetry tags may violate Mojang's terms; the account can be
  actioned. The core never sends a purchase without `confirmed`, never retries, and refuses a repeat of an unresolved one.
- The auth token claims a Windows 10 UWP device while the client is not one; receipts and purchases are attributed to that platform.
- Unverified request shapes (marked guess) hit production services from a real account; reads are harmless, only `transaction/virtual` moves money.

## Purchases setting

The app never sends `store_purchase.v1` unless `store.json` (beside `servers.json`) holds `{"store_purchases_enabled": true}`; a
missing or malformed file means off. With purchases off the whole vanilla flow still runs (offer page, balance check, confirm) and the
send is replaced by the popup "Purchases disabled until verified" (not a vanilla string). The check sits in one place
(`StoreState::dispatch`) and again in the flow, so no path sends while off.

## Render checklist

Every store screen is drawn by the JSON-UI engine from the vanilla `ui/*.json`; data comes from `app/src/store/screens.rs` (globals and
collections) fed by `store_*.v1`. Rows and grids are nested lists: `factory_collection` holds the header item (`TopBar`) then one
item per row; each row's offers, and each offer's info rows and columns, are scoped lists (`name[index].child`). Bindings not listed as
unbound below are populated. Visual pass: for each screen, compare against the expectation and note which listed feed is empty.

Entry: the start screen's "Marketplace" button (`button.menu_store`, always shown) opens the store; Escape or `button.menu_exit`
steps back inside the store, then leaves it for the start screen. Needs a signed-in launcher core; otherwise the store shows the
vanilla connection-failed text.

| Screen | Should appear | Fed by |
| --- | --- | --- |
| `store_layout.store_data_driven_screen`, home | Header bar with "Marketplace", Minecoin balance, search and library buttons; titled rows of offer cards; a trailing "See All" tile on rows that have more; a spinner while loading | `store_home.v1` rows (`factory_collection`, scoped `offer_collection`), `store_balance.v1` (`#coin_balance`), `#page_loading_visible` |
| offer card (all screens) | Thumbnail, title, creator, price line (coin icon, price, strike-through price hidden) or "Owned"/"Free", star rating with score when rated | offer values in `bindings.rs`; thumbnails from `store_image.v1` packed into the menu artwork atlas (`#thumbnail_texture_path`, `RawPath`); info rows/columns scoped under each card |
| row "See All" (`button.show_more_offers`) | Appends the next offers to that row | `store_row_more.v1` with the row's `continuation` |
| same screen, search | Search bar row, then a grid of results; empty-results state; next-page control while more exist | `store_search.v1` (`#search_*`, `#pagination_visible`, `#next_enabled`); results in a `GridList` row (`offer_grid_factory[0].offer_collection`) |
| same screen, offer page | Key art, title, creator, rating, price on the purchase button (or the deactivated button when the balance is short, or nothing when owned), screenshots gallery, description | `store_offer.v1`; `#purchase_*`, `#full_price`, `#main_mashup_key_art_*`, `ItemSummary`/`ImageGallery`/`ItemDescription` rows, `screenshot_collection` |
| `store_inventory.store_inventory_screen` | "My Library" grid of owned offers that resolved to a catalog entry, owned count | `store_entitlements.v1` ids, then `store_offer.v1` for the first 24 (`items_collection`, `#collection_count`) |
| `store_progress.store_progress_screen` | Progress overlay with "This shouldn't take long." while a purchase runs | purchase flow `InProgress` (`#tooltip_text`) |
| `popup_dialog.modal_dialog_popup` | Bundle confirm, insufficient funds ("Get Minecoins"), price mismatch, pending, generic failure with error code and correlation id, purchases disabled, sign-in required | `PurchaseFlow::modal` with vanilla `store.popup.*` texts; the success is a toast text, not drawn yet |

Deliberately unbound (each keeps the layout's own default or stays hidden): text colours, fonts, scales and offsets of the offer info;
badges and icon overlays; genre/language/player-count/tag buttons; wishlist, share, video and rating submission; filters and sorting;
sidebar navigation and nav-button rows; the search box's text entry (the box is inert, so searches run with an empty term);
download and play buttons on owned content; coin bundle purchase (`coin_purchase`) and "Get Minecoins"; the bundle warning
(`bundle_purchase_warning`) and Marketplace Pass error (`csb_purchase_error`) screens; hero and carousel rows; timers and banners.
Unbound visibility flags read false on these screens, as the vanilla controller answers them.

Assumptions to check on screen: a card's row and position are read from the hit key's bracket indices (row = first index minus the
header item); popups replace input on the base screen; a `GridList` needs the single-item `offer_grid_factory` wrapper.
