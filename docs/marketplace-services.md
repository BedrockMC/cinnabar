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
| Row continuation | `POST /api/v2.0/layout/items` body `{continuationToken, inventoryVersion}` | ref path; not wired yet |
| Search (marketplace) | `POST` search-layout body: filters (`price`, `rating`, `...FilterTag` tag filters, range `{type,value,high,low}`), `installedPackIds`, `filterCurrentRealmsPlus`, `filterPastRealmsPlus` | ref member names only; core uses the catalog search below |
| Catalog search | PlayFab `POST /Catalog/Search` (`/Catalog/SearchStores` for store scope) | ref path; body is the PlayFab economy search filter. Core uses the documented `Catalog/SearchItems` through `go-playfab` (doc) |
| Offer detail | PlayFab `/Catalog/GetPublishedItem`; core uses documented `Catalog/GetItem` via `go-playfab` | ref / doc |
| Ratings | read: PlayFab `/Catalog/GetItemReviewSummary`, `/Catalog/GetMyReview`; write: PlayFab `/Catalog/CreateOrUpdateReview` or store `POST /api/v1.0/catalog/reviewitem` body `{ItemId, Rating}` | ref; core returns `Item.Rating` only, no rating writes |
| Minecoin balance | `GET /api/v1.0/currencies/virtual/balances` | ref. `result.virtualCurrencyBalances: [{type, amount}]`; `type` also takes a PlayStation token value |
| Entitlements | `GET /api/v1.0/player/inventory?includeReceipt=true` (the query is 20 characters in the client; the flag name is a guess) | ref path; result shape guess, parsed leniently for id members |
| Inventory refresh | `POST /api/v1.0/inventory/refresh` | ref path; answer `{version}`; not wired |
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
list the bundles and open the platform store; Cinnabar has no platform-store integration, so bundles are shown but the buy button opens
the account page on minecraft.net instead.

## Owned content

Ownership comes from the inventory call; layout requests send every owned id so the service marks rows. Offer `owned` in the control
results is a lookup in that list (refreshed after a purchase). Downloading or decrypting owned packs for worlds is out of scope.

## Risks

- Third-party clients spending real Minecoins with client-supplied telemetry tags may violate Mojang's terms; the account can be
  actioned. The core never sends a purchase without `confirmed`, never retries, and refuses a repeat of an unresolved one.
- The auth token claims a Windows 10 UWP device while the client is not one; receipts and purchases are attributed to that platform.
- Unverified request shapes (marked guess) hit production services from a real account; reads are harmless, only `transaction/virtual` moves money.
