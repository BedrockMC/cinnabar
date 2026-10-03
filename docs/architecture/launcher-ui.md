# Launcher models and client UI

Restructuring step 3 extracts the launcher models first, then the retained UI. The
app remains the composition root and keeps its system order, service workers and
network transports. This migration does not introduce a frame of command latency.

## Ownership

| Crate | Owns | App adapter |
| --- | --- | --- |
| `launcher` | Menu actions and views, screen identities, account state, settings models, local-world forms/progress, store state and installation paths | `MenuRuntime`, account/core processes, filesystem operations and store/world service drivers |
| `client-ui` | Retained HUD/chat/forms/screens, JSON-UI bindings, UI gesture state, item/player previews and presentation | Existing Bevy input, transport, world-query and publication systems |
| `player-state` | The shared composition of inventory authority and local-player facts | `PlayerRuntime` wraps it as the existing Bevy resource |

`UiRuntime` borrows `PlayerState` for projections and commands. It cannot contain
inventory authority or local-player facts. The pre-input presentation snapshot is
still a temporary ledger copy, restored through the existing guard after rendering;
it is not another authoritative inventory owner.

`ui::IconRef` is now the common icon value used by launcher views and presentation.
The view-radius cap is defined once in `render-api` and reexported by
`client-world`. Shared equipment geometry is in `render::equipment`; both the app's
world presentation and the UI preview use those same formulas. The crossbow icon
frame helper is in `inventory` and remains shared with item-use prediction.

## Temporary seams and ordering

The app's existing modules reexport the extracted APIs, limiting churn for the
later gameplay, session and presentation extractions. These adapters deliberately
remain synchronous:

- `MenuScene` supplies visibility, current view, screen and world/panorama policy
  at the old observation points. Launcher actions still reach the existing app
  navigation and service drivers.
- Inventory ingress still drains in the authority phase. Damage closes eligible
  screens in that phase through the same scene policy.
- `prepare_ui_runtime` observes world readiness, local physics, crafter block state,
  item-use animation frames and nametag picking before outbound actions. The new
  crate accepts those observations, including `ItemIconFrames`, and captures the
  same inventory snapshot.
- `publish_ui_runtime` consumes that capture after input enqueue. Rendering uses
  `render_prepared_ui`, then the app performs the same hand binding, scene publish
  and fatal-error handling.
- Form/chat/inventory network flushes retain their queue order, backpressure and
  session checks. UI sound requests keep their bounded queue and are drained at
  the original audio phase.

`client-ui` still depends on `render` for existing draw and preview types, and on
`client-world` for existing read-only world queries. Its extension chrome retains
`mod-host` and `server-experience`. These are explicit temporary dependencies in
the policy, to be narrowed by the later presentation/session migrations. Launcher
models and player state have no Bevy or renderer dependency.

## Enforcement

`tools/architecture/policy.toml` records the new dependency allowlists and rejects
transitive Bevy, renderer and client-UI backedges from launcher and player state.
It rejects app, runtime, session and gameplay dependencies from client-ui. Source rules reject
app module access from the new crates, UI-owned inventory/player authority, and
movement access to UI (including the new crate name).

Shared app test fixtures require client-ui's non-default `test-support` feature.
The gate rejects that feature in production dependencies or feature forwarding,
including renamed and workspace-inherited dependencies. Module alias checks are
scoped to each owning Cargo crate so an identically named alias in another crate
cannot hide a forbidden edge. Protected production modules cannot use `#[path]`
remapping, including conditional attributes; test-only remaps remain allowed.
The source gate checks syntax and aliases, without expanding arbitrary macros.
Marker checks follow the moved producer crate and
still reject undeclared or duplicate identifiers. Boundary regression tests cover
these cases.

## Behavior evidence and references

This is a relocation of existing behavior, not a new parity claim. The source
migration audit retains all 624 original UI test names and all 216 launcher,
local-world, store and installation test names. Tests needing app resources or
services remain app integration tests; pure projections and reducers move with
their owner. The launcher constructor comparison checks that shared view defaults
still match the actual runtime for both visible and hidden menus.


See [UI-edit executable build measurements](../evidence/launcher-ui-build-timings.md).
No game server or client session is used for validation.
