# Menu input and gameplay rendering

The hotbar read Bevy's accumulated wheel independently of the semantic router. Its
focus check covered chat, containers and forms, but omitted MenuRuntime. The
animated first-person rig likewise had no menu/background check. World queues
already stopped under the panorama; the hand used a separate render pass.

Screen input absorption and game visibility are separate flags. Pause absorbs
input while allowing the world and hand. A screen which disallows rendering the
game suppresses world queues and both first-person adapters. A covered menu layer
with `render_only_when_topmost` does not draw. Flags are read from resolved roots,
using the same catalog, inheritance, variables and server overlays as rendering.

## Reference evidence

Lens, reconstructed **1.26.50.26 client**, artifact 6, raw source-backed reads:

- RVA `0x4d143f0`: UIControl's ScreenSettings component lookup, identified by its
  embedded function signature.
- RVA `0x503ca90`: root-component input absorption, mask `0x08` at offset `0x18`.
- RVA `0x50382d0`: root-component game visibility, mask `0x04` at offset `0x18`.
- RVA `0x503d740`: root-component topmost-only rendering, mask `0x02` at offset
  `0x1a`.

These three getter identities are inferred from matching component access and
masks in the named 26.30 reconstruction, rather than authoritative current symbols.
`source_search` found their calls to the ScreenSettings lookup; `artifact_function`
verified the raw bodies. Literal property-name searches in the current corpus did
not return the factory. The named Lens 26.30 functions corroborate the meanings:
`UIScene::absorbsInput` (`0x10284e9c0`), `UIScene::renderGameBehind`
(`0x10284a710`), and `UIScene::renderOnlyWhenTopMost` (`0x10284f470`).

Lens 26.30 `SceneStack::forEachAlwaysAcceptInputScreenWithTop`
(`0x10280ac30`) delivers input to the top scene first, then only to other scenes
which always accept input. `ScreenView::_handleDirtyVisualTree` routes scroll
movement to a scroll-view control through `_sendScrollEvent`; the event belongs
to the UI, independently of hotbar selection.

The grep-able 26.30 reconstruction corroborates this:

- `R:u/UIControlFactory.cpp:3346`: `render_game_behind` defaults to true.
- `R:u/UIControlFactory.cpp:3348`: `absorbs_input` defaults to true.
- `R:u/UIControlFactory.cpp:3391`: `render_only_when_topmost` defaults to true.
- `R:s/SceneStack.cpp:2126`: top-scene input delivery and always-accepting scenes.
- `R:s/SceneStack.cpp:1758`: visible-screen traversal.
- `R:s/ScreenView.cpp:1610` and `R:s/ScreenView.cpp:15377`: UI scroll dispatch.
- `R:u/UIScene.cpp:1067`: root ScreenSettings game-visibility getter.

The read-only retail **v1.26.50.4** resource pack establishes the actual roots:

- `ui/hud_screen.json:3578` explicitly sets `absorbs_input=false`.
- `ui/settings_screen.json:72` inherits `settings_screen_base`, then
  `settings_common.screen_base` (`ui/settings_sections/settings_common.json:2347`),
  `dynamic_dialog_screen` (`:1856`), and `common.base_screen`
  (`ui/ui_common.json:6345`).
- `ui/pause_screen.json:1136` inherits `common.base_screen`; its retail background
  has alpha 0.1 (`:1182`). It omits `render_game_behind`, retaining the true default.

**The supplied Settings JSON does not set `render_game_behind=false`.** Both
Settings and Pause inherit the factory's true default. The implementation preserves
that value instead of inventing a pack property. Cinnabar's existing Settings
panorama is an independent full-screen replacement background; while shown it
also suppresses gameplay, including the hand. Pause and Death do not use that
panorama. This fixes the reported layering without changing the pack defaults.

## Raw input audit

Hotbar wheel selection, cursor capture/re-capture, middle-click pick block,
world Q/Control+Q drops and right-click book opening, and extension keybinds use
the shared absorption check. The semantic authority uses that same check.
`semantic_controls/physical.rs` samples devices for the router; UI readers in
menu, chat, containers, forms and sign editing deliver input to their own screens.
`camera::movement_axes` is test-only. F2 screenshots and the F3 diagnostics toggle
are application tools, rather than gameplay actions.

Offline snapshots exercise the real UI carrier. They do not capture the separate
GPU hand pass; regression tests check hand admission and clearing. No server
connection or native visual parity gate is closed by these offline checks.
