# Native dropped-item rendering

Reference: current `1.26.50.26` reconstruction at mcsrc revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, matching Lens client SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.
The pinned runtime pack is selected by `assets/vanilla-source.json`. This is our
own implementation of identified native behavior, not pasted reconstructed C++.
Fallback names below are identified through matching named 26.30 functions.
The packet-spawn corroboration is explicitly attributed to that older named
reference; the renderer and constructor constants are current-client witnesses.

## Actor origin and animation


The rendering caller is equally important: `ActorRenderDispatcher` at
`0x01fb6fa0` calls `Actor::getInterpolatedRidingPosition` at `0x01c0db40`.
For an unmounted item that calls `getInterpolatedPosition`, `0x01c35c20`, which
interpolates current and previous StateVector XYZ directly. The dispatcher at
`0x01fb3460` places the passed XYZ into `ActorRenderData` offsets 0x10, 0x14
and 0x18; `ItemRenderer::render`, `0x05e54ba0`, translates by those values
without subtracting the item offset. The alternate dispatch at `0x01fb7320`
also builds that direct camera-relative origin. This establishes a native
item origin above collision feet, rather than an extra invented mesh lift.

Cinnabar keeps feet in the actor store, for collision boxes and brightness
sampling. The shared `protocol::ITEM_ACTOR_NETWORK_OFFSET` normalizes item
spawn and absolute/delta movement into that same space. The dropped view
restores the offset exactly once before renderer animation; first-render
camera-distance capture restores it on the current pose too. Delta packets
retain `NetworkOffset` because native `MoveActorDeltaData::parseDeltas` merges
positions into the previous absolute data.

The item constructor draws one uniform bob phase over a full turn.
`ItemRenderer::getRenderYOffset`, `0x05e537c0`, uses time
`(ageTicks + partialTick) * 0.05`, sine angle `2*time + phase`, amplitude 0.1
and baseline 0.1. Ordinary block models additionally rise by 0.2. The Math
table initializer `0x0296ccd0` samples `sinf(index / 10430.3779296875)`;
lookup truncates `angle * 10430.3779296875` and masks to 16 bits.

On the first render only, an actor whose current native origin is less than
one block from the camera captures `80 - cameraYawDegrees`. For that lifetime,
the phase is attenuated by `clamp(2*time, 0, 1)` and a 0.4-block rise eases to
zero over 0.4 seconds. `easeInCubic` at `0x02966360` establishes this as
`0.4 + (0 - 0.4) * fraction^3`, not the cube of the remaining fraction.
Spin is `max(time - 0.025, 0)` radians; the random bob phase is not added to
spin. The captured camera yaw contributes using the native degree-conversion
constants and multiplication order. Walking closer later never recaptures it.

## Geometry, copies and texture selection

`ItemRenderer::render` uses one, two, three and four copies for stack counts
below 2, below 6, below 21 and at least 21 respectively. The renderer
constructor, `0x05e51fd0`, owns its random XYZ copy table, shared across actors;
the first copy stays centered. `_renderItemGroup`, `0x05e54570`, translates
later copies by that table times `0.2 / groupScale` inside the spinning and
group-scaled frame, then applies actor render scale. Thus all XYZ axes spread,
spread rotates with the item, and actor/default-model scale does not stretch it.

The ordinary flat-item route uses group scale 0.3 and shared
`ItemInHandRenderer` `0x04f92b00` with the dropped-item flag, not a hand grip.
Default sprite transform `0x04f9a2e0` applies its 1.5 scale, pixel translation
and rotations to the raster frame. Combined with `TextureTessellator`,
`0x064a0e80`, this maps a raster point `[column, depth, row]` to local
`[0.5 - column/maxSide, 1 - row/maxSide, -depth/maxSide]`; net group scale is
0.45. The texture's longer side controls scale, not a separate width/height
stretch. Pixel-depth mode gives a 1/16 local slab. Alpha values zero and one
produce no extrusion edges, and the rear face samples the same pixel rather
than a separately flipped image. Static item-frame/campfire placements retain
their existing mesh path and do not inherit this dropped-item transform.

Ordinary block drawing at `0x05e53930` uses the dropped display mode. Default
block display `0x07042580` has pixel translation `(0, 3, 0)` and scale 0.25;
the mode correction `0x07042ce0`/`0x070430a0` translates down by 3/16, canceling
that default translation. Centered unit cubes therefore use scale 0.25.
Known cubes reuse the compiled carried-face sheet when its manifest matches
the world catalog, rather than projecting a GUI cube thumbnail or borrowing
biome-tinted terrain faces. Grass soil and side colors consequently keep the
authored carried appearance. See [carried-block-textures.md](carried-block-textures.md).

The native legacy tessellation route queries the item's animation frame before
its icon. Loaded crossbow sprites share the same `getAnimationFrame` /
`getIconInfo` selector as HUD and inventory icons, using canonical charged
projectile NBT rather than damage metadata. Resolved identifier and variant
are part of the dropped model cache key. See [crossbow-use.md](crossbow-use.md).

## Verification and remaining boundaries

Synthetic regressions cover copy-count thresholds, shared isotropic offsets,
stable lifetime phase, actor render scale, half-tick spin, empty/invalid actors,
consistent spawn/absolute/partial-move origins, feet-anchored boxes and
brightness, native sine lookup, first-render camera capture/easing, rotated
unscaled spread, sprite floor origin, non-square sprites, rear UVs, alpha-edge
admission, carried-face colors and shared loaded-icon selection.
Fresh offline vanilla-BDS frames on macOS/Metal at Retina scale 2 show an
ordinary Dirt cube (`2026-10-02_00.28.21.png`) and a dropped 21-Diamond stack
(`2026-10-02_00.24.26_1.png`). The cube uses all six carried faces; the diamond
uses extruded pixel geometry and overlapping stack copies rather than a flat
inventory thumbnail. Floor placement, bob/spin, legibility, geometry, clipping,
depth/layering, scale and colors were inspected on a locally lit test pad.
Walking over each removes the ground actor. These are live functional/device
frame checks, not a version-matched native gallery or pickup-trajectory parity.
Inventory pickup authority is separately covered by the normal transaction
receive path; seeing an actor disappear alone does not establish its count.

Incomplete: shield (`0x05e56560`), banner (`0x05e553b0`), decorated-pot
(`0x05e55da0`), exceptional non-cube blocks and authored custom dropped display
transforms require their native model routes, not a flat GUI thumbnail. This
change corrects ordinary sprites and cubes but does not close those gates.
Multi-layer/enchantment material behavior, native actor lighting/shader parity
and non-default texture-depth data remain incomplete. The existing dropped
shader's directional shade and alpha threshold were not replaced by this
geometry correction. Visual RNG ownership and distributions match the source,
but the local generator is ours, not the native generator; platform sine
rounding is not claimed bit-identical. Existing pickup flight interpolation
is retained, with bob fading out during collection, and is not a completed
native pickup-trajectory reconstruction.
