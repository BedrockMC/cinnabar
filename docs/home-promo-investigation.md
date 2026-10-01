# Start-screen promo investigation

The left-hand promo is still incomplete. The verified change sends the selected
UI language to player messaging; it does not add a guessed placement or control.

## Owner evidence

The read-only core log at `.local/logs/core.log:4546` records a partial home refresh
at `2026-10-01T16:26:48.065+03:00`. Its sole error is the gathering public-config
request returning 404. Earlier partial refreshes show the same error at lines
2453, 2861, 3244, 3643, 3952 and 4364. Messaging failures would be listed separately
by `catalog.HomeFeed`, so these records indicate a successful messaging request.
They do not establish the response's message count or image-download success.

The non-token catalog cache's home value contains 22 messages: one
`MarketplaceButton` and 21 `InboxMessage` entries. It contains no separate promo
message. Only home message fields were inspected; authentication token files were
not read. No account identifiers or cached payloads are included here.

## Vanilla reference

Lens's reconstructed Windows client `1.26.50.26`, artifact 6, provides these
source-backed references. RVAs identify the functions in that artifact:

| Contract | RVA |
| --- | --- |
| Messaging POST builder: Authorization, Session-Id, Content-Type and active Accept-Language | `0x54497e0` |
| Refresh path `/api/v1.0/session/refresh` | `0x5450250` |
| Refresh JSON: `sessionId`, `continuationToken` | `0x544a140` |
| Start path `/api/v1.0/session/start` | `0x54500b0` |
| Start JSON: `sessionId`, `previousSessionId`, `continuationToken` | `0x5449fa0` |
| Event JSON: `SessionId`, `continuationToken`, `events` | `0x544a280` |
| Message surface controller construction | `0x542f5f0` |
| Fetched images associated by message and image ID with a local Core::Path | `0x5495400` |

The refresh request uses the discovered messaging service URI and has no
placement, platform or locale query. Locale is an HTTP header. The service's
registered surfaces are `LoginAnnouncement`, `MarketplaceAnnouncement`,
`MarketplaceButton`, `PlayButton`, `InboxMessage` and `ToastNotification`.
This investigation did not establish whether Cinnabar must send a messaging
start request before its first refresh, or the identity used for the HTTP
Session-Id header. Those behaviors were not changed speculatively.

The supplied `v1.26.50.4/full/resource_pack/ui/start_screen.json` confirms the
existing main-button banners:

- Lines 1626–1682 define `main_button_banner` and its variable bindings.
- Lines 1782–1788 select `#play_button_banner_*`.
- Lines 2101–2108 select `#store_button_banner_*`.
- Lines 944–1013 define the separate left-hand gathering badge/button panel.
- The featured-world control at lines 1869 and 2316 is unrelated to messaging.

No matching persistent promo control or binding was identified in that pack or
the reconstructed client. A read-only inspection of the locally available iOS
`1.26.50.04` OreUI bundle also did not identify a home news carousel. These
findings do not rule out another platform, treatment or service-delivered layout.
The exact promo surface, image key, Learn More action and geometry need an
identified reference before a promo fixture and visibility fix can be written.

## Pipeline and implemented change

The pinned `playermessaging.Client` refreshes the feed. `catalog.flatten` retains
well-formed top-level messages without a surface whitelist. The launcher downloads
their images, caches home data and serves it through `home.v1`. The bridge and
protocol facade preserve the messages and image paths. Rust polls and refreshes
the menu view, but maps only Play/Marketplace art and inbox messages. It has no
general message-button click handler.

The upstream messaging HTTP client omitted Accept-Language. The Rust launcher
now passes its already selected language, converted to BCP 47, to the Go core.
The messaging HTTP adapter supplies that language on refreshes and event reports
without mutating the caller's request. The existing English service default has
one Go constant shared with the store's fallback.

Offline tests use a synthetic transport and token source. They check locale,
authorization, request paths, absence of query filters, continuation, impression
metadata and request cloning. Removing the locale setter makes the regression
fail with an empty Accept-Language header; restoring it passes. A Rust launcher
argument test checks `pt_BR` becomes `pt-BR`.

No live Microsoft, PlayFab or Xbox calls were made. No tile-present snapshot or
before/after visual acceptance is claimed. The missing reference must be resolved
before this investigation can close the promo bug.
