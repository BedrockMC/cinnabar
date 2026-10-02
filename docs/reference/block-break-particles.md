# Block destruction particles

Reference: current `1.26.50.26` mcsrc reconstruction revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, matching Lens client SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`, and
the vanilla particle pack pinned by `assets/vanilla-source.json`.
This implementation is written from the identified contracts, not copied C++.

## Identified destruction path

Current level-event dispatcher `04.cpp`, RVA `04e83890`, handles break events
2001 and 2021 by resolving the block, centering the emitter at the floored cell
plus 0.5, and reading `BlockDestructionParticlesComponent`'s particle count.
Its current getter at `0a670640` returns **100** without a count override.
The named 26.30 getter at `0b18da90` corroborates that default. The previous
Cinnabar value, 32, was not sourced from this path.

`addTerrainParticleEffect` at `04e95ee0` selects `minecraft:block_destruct`.
`_addTerrainEffect` at `04e96080` supplies the count, its cube-root intensity
(matched PE float `DAT_14feff29c`), velocity scalar 1 and radius 0.5 for these
events. It refuses a request only when the selected effect already has more
than 20 emitters or more than 500 particles. These are strict pre-spawn checks,
not a clamp on the admitted burst, and do not count unrelated effects.

The destruction-texture getter at `0a6706c0` first checks an explicit component
texture; its material fallback selects `down`, then `*`. The built-in texture
resolver `04e95920` ordinarily selects texture group zero. The block-graphics
loader `069f19a0` and setter `069e9fc0` populate that group from `down`, not
`up`. Thus ordinary grass uses dirt pixels without a grass tint. There is no
reason to invent a green grass particle or choose a top face based on tint flags.

The pinned `particles/block_destruct.json` remains authoritative for piece size,
random quarter-tile sampling, random direction and speed, lifetime, gravity,
drag and collision. Those formulas are evaluated by the existing pack particle
engine rather than replaced with a hand-tuned visual effect.

## Implementation and verification

`particles/tiles.rs` selects the resolved bottom-face material and its tint.
`render::block_break_request` supplies the native default count and intensity.
`ParticleSystem::spawn_terrain` applies effect-local pre-spawn admission checks.
Regressions cover sequential and hashed grass IDs, untinted dirt selection,
100 live pieces from a valid effect, exact emitter/particle threshold boundaries,
and independence from other effects. Focused suites pass locally.
Fresh macOS/Metal Retina-scale-2 grass destruction frames show the dense brown
particle burst. The owner manually tested and accepted block breaking. This is
functional visual acceptance of the reported bug, not a matched native parity gate.

Incomplete: custom destruction-component count/texture overrides, the special
built-in block texture-group/crop branch, random block texture variants and
seasonal color overrides. Crack effects are a separate contract: native
`04e8fe80` selects a random point in the block's current visual bounds and moves
it outside the hit face, then requests count 1, velocity 0.7 and radius 0.
Upstream integration retains the sourced one-piece/0.7/zero-radius crack
parameters and face placement; exact non-cube visual bounds and mining cadence
remain incomplete. The legacy Terrain event routing also needs independent parity
work. These gaps do not justify using the old 32-piece or green-top fallback
for ordinary grass destruction.
