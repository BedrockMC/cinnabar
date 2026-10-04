# Dropped-item and particle lightmap composition

Reference: current `1.26.50.26` mcsrc reconstruction at revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, with the matching client executable
SHA-256 recorded in [dropped-items.md](dropped-items.md). The runtime particle
definitions come from the pack selected by `assets/vanilla-source.json`.
These contracts are implemented in our own Rust/WGSL.

## Current-client lighting contracts

`ItemInHandRenderer::renderItem`, RVA `04f92b00`, is the ordinary dropped-item
tessellation consumer identified in [dropped-items.md](dropped-items.md).
Its actor lighting setup at `0213d220` supplies `(sky, block) / 16` to
`LightTexture::getColorForUV`, `07083ac0`. The latter samples normalized byte
RGB with clamp-linear coordinates `16 * uv - 0.5`. There is no integer texel
shortcut or gamma-to-linear transfer inside this lookup. Cinnabar's existing
`actor_light_colour` reproduces the lookup with a transposed table: block in
the low nibble, sky in the high nibble.

Modern pack particles use the same RGB lookup. Current
`ParticleSystemEngine::tick`, `02450320` (`__unmapped/02.cpp`), constructs its
16-by-16 gameplay-light cache by calling `07083ac0` for each pair of nibbles
multiplied by `0.0625` (matching executable float `14ffa90e0`).
`ParticleEmitterActual::getGameplayLightForParticle`, `02446b90`, retrieves
the four-channel cached entry for the particle's brightness pair.
`getBrightnessPairForParticle`, `02446c00`, floors world particle coordinates,
including the emitter translation for local-position effects. The pinned
`block_destruct` effect enables `minecraft:particle_appearance_lighting`.
An effect without that component bypasses gameplay light.

The current component registration (`06c9dc10`, `__unmapped/06.cpp`) resolves
the appearance-lighting factory (`06cb8310`) and update dispatch (`06bb6f20`).
The update multiplies the cached gameplay RGB into render color, leaving alpha
untouched. This verifies the modern component path, not just the older named
`ParticleAppearanceLightingComponent` navigation reference.

The ordinary native material color contract composes normalized gamma RGB;
the already sourced actor pipeline applies the same contract. Bevy samples
our sRGB atlas as linear RGB and writes an sRGB target, so the item/particle
shaders undo the atlas decode, multiply native tint and RGB lighting, then
convert the completed color to linear once at output. Item overlays precede
the lighting product and item distance fog also composes gamma RGB.

## Corrected data path

Dropped items now use the byte-quantized `/16` lookup and native color
composition instead of multiplying gamma lightmap RGB into decoded texture
RGB. The latter caused an extra sRGB encoding of the dim-light multiplier.

Lit particles publish independent block/sky nibbles and a lighting flag in
the existing instance record. Their GPU pipeline binds the shared world
lightmap, so environment, brightness and vision updates affect existing
particles without a separate CPU brightness curve. The old fixed ambient
floor, minimum night sky scale and scalar maximum of the channels are gone.
Authored particle tint stays in gamma RGB until the shader completes the
texture/tint/light product. Unlit effects retain their bypass.

## Verification and remaining boundaries

`item_particle_lighting` exercises the production item and particle vertex
and fragment functions on an sRGB GPU target, comparing pixels against the
native byte lookup in darkness, daylight, sky light at night, torchlight,
minimum/maximum brightness and night vision. Particle draw-list regressions
check separate channels, dark samples, level clamping and unlit admission.

The October 4 user-authorized scratch-BDS run used macOS/Metal on Apple M3 Pro,
2560 × 1504 rendered pixels at Retina scale 2. The user first confirmed the
corrected breaking particles, then accepted dropped items and survival breaking
after the capability correction. The tested debug executable SHA-256 was
`72ac50642fefac549454e898496575339fb75117b1d4ddff123309591c5d60a1`.
This is a live functional/color acceptance of this correction, not a full
version-matched native scene or performance parity gate.

This correction does not close complete item or particle parity. The existing
provisional dropped-item normal shade still needs separate sprite-versus-block
material handling. Native item AABB brightness sampling is not replaced by
this correction. Particle solid-cell sampling (`02445220`) selects the
brightest of six axial neighbors; that fallback remains incomplete in our
point-sampling world adapter. Particle fog/material blending and unrelated
geometry, animation and destruction contracts retain their existing gates.
