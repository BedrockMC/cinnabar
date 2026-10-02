# Local crouch camera

## Native reference

The current client reference is `mcsrc-1.26.50`, reconstruction revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current `1.26.50.26/src/__unmapped`.
Its matching Windows PE has SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`
and image base `0x140000000`. Values below were read from that PE, not inferred
from another edition or the apparent height in screenshots.

The symbols in the retained 26.30 source identify the otherwise unnamed current
functions. Current system registration `07.cpp`, RVA `072b6020`, registers
`VanillaOffsetSystem` on both sides. Its vtable `0x1501466f0`, tick slot `+0x28`,
points to `02.cpp` RVA `02c3f610`.

| Native contract | Current RVA / data | Result |
| --- | --- | --- |
| Version-selected crouch drop | `02c3f610` | Current version selects float bits `0x3eb33333` (`0.35`); the legacy branch's `0.125` is not this client |
| Client offset tick | `02c3f4a0` | Save previous offset, then approach the target once per completed actor tick |
| Tick blend | `DAT_14fec3380` | `0.5`, not per-render-frame damping |
| Standing camera anchor | `DAT_14ffab6b0` | `1.62001002`, the same float already retained as `PLAYER_NETWORK_OFFSET` |
| Horizontal-pose eye height | `DAT_14feff2a0` | `0.4` above feet; takes precedence over sneaking |
| Camera getter | `02c405e0` | Subtract the frame-interpolated previous/current offset from the interpolated anchor |

The corresponding named 26.30 functions are
`VanillaOffsetSystemUtil::_clientTick` RVA `06367980` and
`VanillaOffsetSystem::getCameraPosition` RVA `06368eb0`. The client tick receives
the actor data flags and optional `IsHorizontalPoseFlagComponent`; sneaking is
flag bit 1. `UpdateHorizontalPoseSystem::update` (26.30 RVA `059f53c0`) admits
gliding, swimming and crawling flags. The sleeping branch has a distinct native
`0.2` target; that branch and riding/dynamic offset inputs are not implemented by
this local locomotion change.

Current `SneakTriggerActionSystem` executes through RVAs `0c59ccb0`, `0c59c870`
and `0c581810`. The final action consumes the input start/stop-sneaking bits and
sets/clears actor flag bit 1. Current `SneakingSystem` tick adapter RVA
`0c59f5a0` uses native base multiplier `DAT_150056088 = 0.300000012`; Swift Sneak
adds `DAT_14ffab6c8 = 0.150000006` per level and caps at one. This audit does not
claim the existing collision/Swift Sneak simulation is fully native-conformant.

## Cinnabar implementation

`movement/physics/eye.rs` retains previous/current camera offset and advances it
only after a completed physics tick. Standing, crouched and horizontal pose
targets follow the native priority and tick blend. Frame alpha interpolates the
offset using the same fraction as the retained movement positions. Both held
Shift and a low-ceiling forced crouch therefore lower the eye; release returns
smoothly to standing.

Physics keeps `render_feet_position()` separate from `render_eye_position()`.
`LocalViewPose`, the frozen local-player frame and local-avatar visibility carry
both positions. Interaction uses the lowered eye, while actor placement and
portal feet sampling use real interpolated feet. Outbound movement still uses
feet plus the protocol position offset; lowering the camera never creates an
extra downward movement packet.

Both Shift scancodes (`0xe1`, `0xe5`) were already mapped to semantic `Sneak`;
the movement runtime already applied held/toggled crouch, low-ceiling forcing,
sprint exclusion and wire start/stop edges. The missing path was the rendered
camera height, not the keyboard binding.

## Verification and remaining gate

Focused regressions cover the first-tick and second-tick half-blend, render-frame
interpolation, release, horizontal-pose priority, session/reanchor reset,
low-ceiling forced crouch without a held key, both Shift bindings, fixed outbound
anchor, finite atomic frame publication, separate actor feet and lowered
interaction eye. The focused movement suite passed all 242 tests, camera passed
32 and Phase 4 presentation passed 11. The latter two were run from the same
successfully compiled movement-suite artifact while a separate, in-progress
equipment diagnostic temporarily blocked recompilation. These are
source-contract tests, not a native rendered-frame
comparison. The owner manually tested and accepted the lowered sneak camera
on the macOS/Metal client. A controlled native standing/held Shift/release and
third-person comparison remains incomplete; no broad visual parity gate is closed.
