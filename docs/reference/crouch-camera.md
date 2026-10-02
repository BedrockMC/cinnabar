# Local crouch camera

## Native reference






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
