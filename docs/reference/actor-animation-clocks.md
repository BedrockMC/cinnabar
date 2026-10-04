# Native actor animation clocks

The mob walking fix preserves the pack's authored `anim_time_update` expression and
uses its result as the clip clock. The compiler previously discarded this field, so
procedural quadruped and chicken leg rotations read elapsed seconds through
`query.anim_time`, even though their packs request distance-driven phase. This was a
clock-input mismatch, independent of packet velocity or a leg-rotation constant.

This fixes that identified movement desynchronization. It does **not** close the
broader native actor-animation parity gate: ordinary actors still evaluate Molang
on fixed simulation ticks and interpolate completed bone poses. Vanilla instead
samples interpolated motion queries while applying the animation at render time.
These procedures differ for nonlinear bone expressions, including the cosine used
by walking legs. Start/loop delays, independent instances of a shared clip, and
specialized motion multipliers also remain incomplete; see [plan.md](../../plan.md).

## Reference identity and evidence

The current reference is the derived MCSRC client reconstruction `1.26.50.26`, at
MCSRC revision `da728f0ce4d7a5ae0be443b8abe03119858d923e`. The matching analyzed
executable has SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`
and PE image base `0x140000000`. Addresses in the following table are RVAs; emitted
hashes identify canonical bodies in `current/1.26.50.26/index/functions/*.jsonl`.
The descriptions are our interpretation of those bodies, not copied reconstruction
source. Current function names are non-authoritative navigation labels.

| Behavior | Current RVA | Canonical emitted body SHA-256 |
| --- | --- | --- |
| Authored time expression copied to the skeletal definition | `0x01e3df60` | `6c498d438aef7575e36efa51552abf6eacd1babdee090b20592ca645f91f4a60` |
| Skeletal definition default clock construction | `0x020e9720` | `a62d534f6e596efc5f0cfdde8b43b63adecbf4ba098349c22e37556ebe552a77` |
| Skeletal time update, loop handling, and pose application | `0x020ec2c0` | `79adea1fcd03802fb7a9754e4a84528bc984a003b81199f29f241228021b0d66` |
| Skeletal player reset | `0x020ed2a0` | `ed9f4ce2dc434f80d24985aa04d919b6add87b9392a1ad6860c13478e65bbbc0` |
| Controller transitions and state application | `0x020f0810` | `af734d711d25c48dfcea8dde3c959716ff1a3cfae1423a868fb33e58784c63c6` |
| Controller-state recursive player reset | `0x020f4950` | `a3ac2f7583f698f4accdf1dd8247d74fd76c99af839e65cfe624991f2fb908ed` |
| Modified-distance query getter | `0x024fe160` | `370c969b430494c533648718c7689fbb67502229bac4c9cc08579161fa0238e3` |
| Modified-speed query getter | `0x024fe200` | `a0e500d0b7e9e96355f7e4949665f03d5cd41061f453c8984131078dedd0e006` |
| Walk-animation speed and distance update | `0x070c3ef0` | `25f83fe25138bcc673990454ccb2a376263952b132e5b8f961fde7e3093d4263` |
| State-vector horizontal displacement input | `0x070c4230` | `3e98f9c88c1f0b6cb787ebe0d06bd445fe4011fd512327f7f027fad846ba30c3` |
| Dynamic-render-offset horizontal displacement addition | `0x070c4890` | `ed134b561aea99d27cb3b8010878edfbdd18bed83a54458d140b3e46cc7231cc` |

Named 26.30 `ActorSkeletalAnimationPlayer::applyToPose` (`0x09e87120`),
`resetAnimation` (`0x09e880e0`), and
`HardcodedAnimationSystem::doHardcodedAnimation` (`0x05165ba0`) provided the
navigation leads. The corresponding current bodies and executable data establish
the behavior and constants reported here.

Two monolithic registration functions, schema RVA `0x01e153d0` and query
registration RVA `0x024d3770`, are explicitly unresolved in the reconstruction.
Their catalog strings alone are not canonical-body evidence. Matching-executable
inspection completes the links:

- Schema instruction VA `0x141e18475` loads `anim_time_update` from string RVA
  `0x10632334`. Its callback vtable VA `0x1500dd020` points to the canonical
  `0x01e3df60`, which stores the expression in skeletal-definition offset `0x70`.
  The constructor and pose application use this same field.
- Query registration instructions VAs `0x1424d460d` and `0x1424d46e4` load
  `query.modified_distance_moved` and `query.modified_move_speed`. Their closure
  vtables, VAs `0x150109fb0` and `0x150109fe0`, link to canonical getters
  `0x024fe160` and `0x024fe200` respectively.
- Skeletal vtable VA `0x1500f2040` links pose application and reset. Controller
  state vtable VA `0x1500f2160` links its recursive reset. The controller's
  transition call at VA `0x1420f0b12` invokes the target state's reset slot.

## Time assignment and lifecycle

The definition constructor compiles the default expression
`query.anim_time + query.delta_time`. During pose application, vanilla places the
player's previous stored clip time in `query.anim_time`, evaluates the time
expression, and **assigns its result** to the stored time. It does not add that
result as a delta. Loop/hold handling then updates the stored time and exposes it
to bone-channel Molang through `query.anim_time`.

The current body establishes these endpoint rules:

- Reaching or passing the declared length sets the player's finished flag. This
  flag is sticky until reset; reversing the clock below the length does not clear
  it.
- A looping clip wraps with `fmodf` only when its length is nonzero and its time
  is strictly greater than its length. Equality samples the endpoint. A
  zero-length looping procedural clip keeps its unbounded time.
- Hold-on-last-frame stores the smaller of assigned time and length.
- A one-shot skips pose/event application when time is strictly greater than
  length. Equality still applies the endpoint.
- The assigned expression result is not clamped to zero. Negative clocks remain
  negative, and the remainder operation is not Euclidean normalization.
- Effective blend below float epsilon returns before the clock advances. Its
  previous time and finished flag remain available when it resumes. Start and
  loop delays can also pause clock application.

Reset initializes clip time to zero, previous event time to minus one, finished
to false, and the start-delay field to its initialization sentinel. Entering a
controller state recursively resets its child players. An absolute expression
such as modified distance consequently restores the actor's movement phase on
its next application; it does not subtract a distance baseline for that state.

Cinnabar retains scripted clocks alongside each rig, evaluates their expressions
once before sampling the main and additional model geometries, and supplies the
same resulting time to their bone queries. Dormant zero-weight clocks persist.
Actor resets discard prior clocks before controller finished-condition checks,
and controller reentry uses the new entry tick. The current key of clip plus entry
tick still does not represent every independent native animation-player instance.
Clips without an authored update retain the existing elapsed-tick timing path;
their complete pause/render-time behavior is not claimed as newly verified parity.

## Motion inputs and render interpolation

The walk-animation component contains a base multiplier, previous speed, current
speed, accumulated modified distance, and current horizontal displacement. The
state-vector input uses X/Z displacement between current and previous positions;
dynamic render offsets add their own X/Z displacement. Vertical displacement does
not enter this walking input.

For ordinary moving actors, with horizontal displacement `d`, base multiplier
`b`, speed `s`, and accumulated distance `D`, the verified update is:

```text
previous_speed = s
s = 0.6 * s + b * min(1.6 * d, 0.4)
D = D + s
```

When displacement is zero, the target instead uses
`min(abs(wrapped_body_yaw_delta) * 0.02, 0.2)`. Passengers zero previous/current
speed and stop accumulating modified distance. Hurt/fire states can multiply the
base by `1.5`; jumping multiplies it by `0.35`. These specialized conditions are
not all implemented in Cinnabar's current motion model.

The native query getters use render interpolation fraction `alpha`:

```text
modified_distance_moved = D - (1 - alpha) * s
modified_move_speed = min(lerp(previous_speed, s, alpha), 1)
```

The speed getter applies an additional factor of `1.5` for actor flag mask `0x800`.
The getter's interpolation fraction is at RenderParams offset `0x108`, distinct
from clip animation time at `0x114` and frame delta at `0x118`. Current Cinnabar
ordinary-actor queries use tick motion values before the renderer interpolates
poses; replacing the missing authored clock therefore repairs the phase input but
does not implement these native render-time getters. No teleport distance cutoff
or teleport-specific native animation reset was established by this investigation.

## Pack witnesses and carrier rebuilds

[assets/vanilla-source.json](../../assets/vanilla-source.json) selects the pack
revision, hash, archive, and local cache root; do not copy those pins into animation
code. Within its `resource_pack` directory,
`animations/quadruped.animation.json` (`animation.quadruped.walk`) and
`animations/chicken.animation.json` (`animation.chicken.move`) both author
`anim_time_update: query.modified_distance_moved`, loop their procedural leg
channels, and read `query.anim_time` in leg rotation expressions. Neither needs a
positive keyframe duration to advance this procedural clock.

The carrier now includes the compiled expression index and validates that it
references a retained Molang expression. Its compatibility version is defined
once by `ENTITY_BLOB_VERSION` in
[crates/assets/src/entity.rs](../../crates/assets/src/entity.rs). Bumping that
version prevents a pre-fix catalog, which cannot retain authored clocks, from
silently loading with the repaired runtime. The optional field's serialization
default is useful for payload handling; it does not bypass the version check.

Run `make assets` after updating the compiler/runtime. The actor artwork and
equipment catalogs embed the entity carrier's hash and must rebuild with it;
automatic preparation includes that dependency in their cache identities. The
[Makefile](../../Makefile) owns the carrier path and compiler-input dependencies.
The required entity carrier must fail startup with its path and `make assets`
rebuild instruction if stale or invalid. Packs, compiled carriers, reference
executables, and comparison captures remain local artifacts and do not enter git.
