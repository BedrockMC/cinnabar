# Start-screen promo investigation

The left-hand promo is the gathering panel. Its parity gate remains incomplete:
the offline fixture is authored, and the exact current public-config request and
complete click behavior still need verification.

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
| Session lifecycle: start first, refresh only after start succeeds | `0x54334a0` |
| Event JSON: `SessionId`, `continuationToken`, `events` | `0x544a280` |
| Message surface controller construction | `0x542f5f0` |
| Fetched images associated by message and image ID with a local Core::Path | `0x5495400` |

The refresh request uses the discovered messaging service URI and has no
placement, platform or locale query. Locale is an HTTP header. The service's
registered surfaces are `LoginAnnouncement`, `MarketplaceAnnouncement`,
`MarketplaceButton`, `PlayButton`, `InboxMessage` and `ToastNotification`.
Vanilla starts the session before refreshing. The pinned Go client instead
refreshes immediately. This is another confirmed contract mismatch, still
unfixed. RVA `0x5498290` parses the start response's `result.id` as the session
ID and `reportFrequency` (default 20), then handles the shared messages/inbox
response at `0x544b210`. The pinned Go client cannot update its session ID, so
rewriting only the first request URL would be insufficient. Previous-session
persistence and service-session lifecycle still need verification. The HTTP
Session-Id is separate from the messaging JSON session ID: base request
constructor `0x8667510` copies the service's session string, and POST builder
`0x54497e0` uses it. Supplying the messaging client's ID there would be wrong.

The supplied `v1.26.50.4/full/resource_pack/ui/start_screen.json` confirms the
existing main-button banners:

- Lines 1626–1682 define `main_button_banner` and its variable bindings.
- Lines 1782–1788 select `#play_button_banner_*`.
- Lines 2101–2108 select `#store_button_banner_*`.
- Lines 944–1013 define the separate left-hand gathering badge/button panel.
- The featured-world control at lines 1869 and 2316 is unrelated to messaging.

The gathering panel has the described caption/image/button shape. Its visibility
is `#gathering_enabled`; its image is `#gathering_badge` with
`#gathering_badge_file_system`, caption is `#gathering_countdown_text`, and button
is `#gathering_button_text` with action `button.gathering`. The current controller's
label callback at RVA `0x558aa50` uses configured text or
`gathering.button.liveEventFallback`. Cinnabar already binds this panel from
`LiveEventCard`, but the owner's gathering fetch fails with 404.

The Dungeons II tile's association with that panel remains unconfirmed. A
read-only inspection of the locally available iOS
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

No live Microsoft, PlayFab or Xbox calls were made. The earlier investigation did
not capture a tile-present snapshot or close visual acceptance.

## Desktop gathering correction

The Android/Google query literals were in `core/catalog/home.go`, not the fork.
`core/clientplatform` now owns the desktop identity; the gatherings query and the
common authenticated service token configuration use it. Messaging, persona,
marketplace and discovery do not have equivalent platform query parameters in
their current request builders. Their shared token source already defaults to
Windows, and now receives that platform explicitly from the same constant.
The XAL Android OAuth application configuration is separate from these service
platform fields and has not been changed.

Current Lens source-backed artifact 6 (`1.26.50.26`) has the build accessors at
RVAs `0x94f40` (`Windows10`, nine characters) and `0x94f70` (`Win32`, five
characters). The gathering manager constructor at `0x542a4c0` registers service
name `gatherings`. This corroborates the fork's discovery key; a discovered host
containing `gatherings-secondary` is not evidence for changing that key.
Current controller callback `0x558aa50` localizes the configured button text or
falls back to `gathering.button.liveEventFallback`, which is now also used by the
Rust home-feed mapping. The pack's `texts/en_US.lang:11964` supplies “Join Game”.

The exact public request is positively identified in the older Lens client:
`GatheringManager::_setupRefreshTask()::$_0::invoke`, VA `0x1046e7cd0`, constructs
the GET handler for `/api/v1.0/config/public` and calls
`GatheringServiceRequestHandler::_getRequestUriParameters`, VA `0x1046b7430`.
The latter reads the network game version and the build's platform/subplatform
virtual accessors, omitting the subplatform only when empty. The mcsrc references
are `R:g/GatheringServiceRequestHandler.cpp:33` (query construction),
`R:g/GatheringService.cpp:1944` (host resolution; discovery registration at 1980),
and `R:g/GatheringServiceRequest.cpp:421` / `:469` (Authorization / Session-Id).
The request code sets JSON Content-Type only when it has a nonempty body.
The fork's public-config request uses its supplied Minecraft service token.
These older request details have not yet been matched to a current Lens public
request body; unsuccessful literal searches do not prove it was removed.
The endpoint and authentication implementation therefore remain unchanged.

The binding registration is `R:s/StartMenuScreenController.cpp:2645` through
`:2814`. Its badge callback (`0x10323f390`) returns downloaded art or the built-in
badge. Its file-system callback (`0x10323f450`) returns `RawPath` for downloaded
art and `InUserPackage` for the fallback. Lens data at `0x10f5fc0f0` and
`0x10f5fc0d5` identifies those strings. The current pack's
`ui/start_screen.json:816` binds both texture and texture_file_system;
`:802` binds `button.gathering`, `#gathering_button_text` and button enabled;
`:910` binds `#gathering_countdown_text`; `:1009` controls panel visibility.
Cinnabar was missing the downloaded badge's file-system binding; it now supplies
`RawPath`. Its existing route-to-Servers/direct-connect action is provisional:
the full vanilla controller action, default badge, GIF and countdown behavior
are not claimed as complete.

The offline Go transport test checks the desktop query, discovery host and token,
then maps the synthetic public response into `Home.LiveEvents` and cached artwork.
It can export that actual Go home-feed JSON using `CINNABAR_HOME_PROMO_FIXTURE`.
The Rust mapping test checks configured and fallback labels. The optional real
carrier snapshot test consumes the Go feed and renders a generated blue badge;
it asserts that badge pixels reach the start screen. Generated JSON, art and PNG
stay in the scratch directory. This is not a recorded-response fixture and does
not establish that correcting the query resolves the owner's service 404.

## Verification interruption

Full `rcheck` was started for code commit
`0ddbf259ce59576d1399e86611c139208ac72fb9` using `RCHECK_SOURCE_MODE=objects`,
which avoids stash and remote ref updates. Formatting passed; clippy began
compiling but the remote pod entered `Failed`. Subsequent SSH polls returned
`cannot exec into a container in a completed pod; current phase is Failed`.
The local poll was interrupted with exit 130. There is no green rcheck result;
architecture, Rust tests and the remaining remote gates are unverified. Focused
offline Go tests passed locally. No new PNGs could be captured.
