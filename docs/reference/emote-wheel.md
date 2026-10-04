# Emote wheel and custom playback

Cinnabar opens the native JSON-UI wheel with the remappable `key.emote` control.
The defaults are keyboard B and controller D-pad Left. Digits 1–4 choose its top, right, bottom and left
slots; pointer selection uses the authored radial control's geometry and dead zone.
The control is listed in Keyboard & Mouse and Controller settings. Change Emotes equips the
original custom dance in a chosen slot and persists the complete slot array.
Equipping an already equipped piece moves/swaps it rather than duplicating it.

## Identified references

Lens's 26.30 reference and the current reconstructed Windows client identify:

- Keyboard defaults: `VanillaClientInputMappingFactory::_populateKeyboardDefaults`
  at `0x10096bc20` and full layout at `0x10096e1f0`, action `0x34`, keyboard B.
  The current Windows mapper at RVA `0x7395f10` names that action `key.emote`.
- The pinned pack's `persona_emote.emote_wheel_screen` and
  `persona_common.emote_wheel_panel`: four cardinal slots. The controller's
  contextual global bindings query each originating control's `#index`.
- Controller defaults: `createInputMappingTemplates` at `0x100963250` maps
  action `0x34` to native button 7, identified as D-pad Left by the sprite/name
  initializer at `0x102e15a30`. The controller mapper at `0x10097ef10` binds it
  to `button.emote`. Supplemental binding rows are appended to preserve saved IDs.
- `LocalPlayer::playEmoteSlot` at `0x103d26d00`: accepts slots 0–3.
  `PersonaAppearance::setEmote` at `0x103c3b260`: bounds slots and swaps an
  already equipped piece into the requested slot.
- `SelectionWheelComponent::receive` at `0x1024ea3d0`: radius follows the shorter
  rectangle side; the inner boundary is excluded and the outer is included.
  Visibility at `0x1024e9f30` selects one state, and construction at `0x1024e9af0`
  starts with no hovered slice.
- `LocalPlayer::playEmote` at `0x103d24780` and current Windows
  1.26.50.26 RVA `0x4f382c0`: animation controller playback and emoting status.
  This function does not directly change the camera perspective.
- `PlayerMovement::shouldStopEmoting` at `0x1065106b0` and
  `ClientInputUpdateSystem::updateStopEmotingRequest` at `0x1022bc6a0`:
  nonzero horizontal movement requests cancellation.
- `HudPlayerRenderer::update`, current RVA `0x9c78c70`: emoting keeps the paper
  doll visible with the existing hold timer.

## Original custom emote

Twerk is an owned animation authored from observing the public
[Lunar product preview](https://store.lunarclient.com/products/3720241/twerk).
The listing identifies a repeating dance that stops on movement. The shared
custom catalog owns its loop period, identifier and label; no Lunar animation
files or Marketplace entitlement identifiers are included.

The revised owned clip follows the public video's sustained deep squat, wide
stance, level head, hands beside the thighs and hip pulse. Model-space joint
targets rotate the torso about the hips even on independent, shoulder-pivot
skin bones; clothing still follows its actual parent hierarchy. Root Y/Z offsets
anchor the leg bottom-face centers through the loop. Torso counter-tilt keeps
shoulder/head height steady as the pelvis rocks; varying spread keeps the foot
centers fixed laterally as well. Rigid tilted foot corners
are not articulated soles, and exact visual parity remains incomplete.

The user rejected the initial steady-head revision as still reading as a crouch.
The next owned revision doubles the leg-driven hip excursion, makes the torso
more upright, adds a small alternating torso twist, and brings the hands toward
the thighs. Foot centers and head height remain anchored. This is still an
approximation on the existing rigid limb model, not verified Lunar keyframes
or an articulated knee rig. The subsequent knee revision below supersedes this
rigid-leg approximation for supported skins.

Classic cuboid skins now receive a temporary render-only thigh/shin split during
playback. Side-face UVs are cropped at the knee rather than stretching or repeating
the whole leg texture; mirrored legs and clothing retain their source mappings.
A two-segment solve bends the knees forward while keeping foot centers planted.
The pelvis pulses beneath steady shoulders. Source models, tick-owned poses,
first-person hands and remote actors keep their original skeletons; stopping the
emote restores the original mesh and pose. Unsupported rotated/polygon/custom
limbs retain the rigid approximation. Rigid armor knee articulation and exact
Lunar animation parity remain incomplete. Live acceptance of the knee revision
is pending.

Sampling resolves named bones in the player's actual geometry and applies
channels before skeleton composition. Body, clothing/persona layers and armor
share that pose. The native first-person hand and other actors keep their normal
animation. Render time drives the loop independently of simulation tick speed.
Movement, jump, attack/use, other modal screens, loss of focus, and actor/session
reset cancel local playback. F5 perspective remains user controlled.

This is a client-local feature. Standard Bedrock clients and servers do not know
its custom clip, so no fake native emote packet is emitted. Exact Lunar keyframes,
native Marketplace ownership/playback, and remote custom-emote synchronization
remain incomplete. The small custom equip flow substitutes for native Dressing
Room navigation and does not close that parity gate.
Analog-stick and touch wheel navigation remain incomplete; controller navigation
currently uses the D-pad and confirm/back buttons.

## Verification checkpoint

The complete JSON-UI suite passes (480 tests), including native wheel state and
indexed bindings plus square-panel sizing at different aspect ratios. Focused
tests cover saved slots/bindings, skeleton/clothing sampling, native wheel/equip
rendering and radial hits, GUI player vertices, current-frame cancellation,
same-frame input consumption, controller opening, session/focus retirement,
and production actor publication with native hand/remote ownership preserved.
Eleven custom-animation tests pass, including native joint landmarks, foot-center
anchoring, attached arms/head and equivalent flat-biped/clothing retargeting.
The twist regression also checks level head ownership and planted feet.
Knee regressions check connected equal-length thigh/shin segments, planted feet,
steady head height, texture cropping, mirroring, clothing and source immutability.
Controller regressions also cover equipping/playing the left slot without
retriggering the wheel opener, and closing-frame inventory consumption followed
by fresh real drop/book actions.
Software-rendered 1280x720 DPI-1 wheel/equip frames were inspected for readable
text, complete geometry and clipping. This does not replace the required live
Windows game pass; that acceptance is pending and the feature remains local.
Strict all-target clippy passes for all six changed packages. The required
`verify-affected --base origin/dev` run passes formatting, architecture and
compilation, then stops at the unchanged meshing regression
`compiled_ice_over_water_preserves_still_source_and_flat_top` in
`crates/meshing/tests/liquid_barriers.rs` (the touching liquid-face assertion).
That failure also occurred before this feature; emote work does not modify meshing.
