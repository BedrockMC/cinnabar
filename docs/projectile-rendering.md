# Projectile rendering investigation

The fixes in `fix/projectile-render` address invisible item sprites, undersampled arrow
textures, discarded arrow plane backs, duplicated world yaw, and dropped remote motion.
They do not close the vanilla projectile parity gate.

## References



The pinned resource pack is selected by `assets/vanilla-source.json`. Its
`models/entity/item_sprite.geo.json` declares an 8×8 UV frame on an 8×8×0 cube,
origin `[-4,-2,0]`: 0.5 blocks square, from -0.125 to 0.375 blocks vertically.
The entity definition directly binds its item icon, such as
`textures/items/ender_pearl`, rather than an inventory atlas lookup. The actual
icon raster can be 16×16. The compiler previously excluded item texture paths,
required equal declared/raster dimensions, and rejected the unused box UV envelope.
These were independent barriers to publishing the sprite artwork.

The pack's arrow geometry has two crossed shaft planes and a cap. Shaft face UVs
omit `uv_size`; their default region is 16×5 texels. The arrow animation supplies
pitch/yaw and axis scale `[0.7,0.7,0.9]`. Item sprites use
`animation.actor.billboard`; the compiler continues to load the pack's geometry,
texture selection, and animation rather than synthesizing replacements.

The neutral catalog profile already declares two-sided rendering. Its mesh builder
now supplies front UVs for a missing back face, while retaining explicit opposing
UVs and leaving the separate skin mesh builder unchanged. This repairs arrow backs
within that profile. Exact target arrow material and lighting remain unverified.

## Verification

Regression tests were executed against unfixed production code and observed failing
before each corresponding implementation change. They cover item icon admission,
raster resolution, sprite UV bounds, default arrow face UV size, arrow back UVs,
absolute arrow target yaw, duplicate world yaw, remote motion ingress, and initial
arrow motion rotation. Later motion must retain the existing orientation and position.

`projectile_report::render_projectile_states` uses the real compiled vanilla geometry,
artwork, actor poses, presentation adapter, and scene publication. `scene_report` then
rasterizes the published triangles offline, including backface UV selection. The
fixed-camera gallery covers arrow flight, an arrow at a textured stone block, pearl,
and snowball. This is a CPU frame witness, not target-platform GPU acceptance.

Rebuild the actor carrier with `make assets` before using the fixes with previously
compiled local assets. This work does not write the owner's `.local` directory.

## Open parity checks


No live server connection was used and no parity gate was marked complete.
