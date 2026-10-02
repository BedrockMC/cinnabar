# Native arrow entity rendering

References: the current `1.26.50.26` reconstruction at revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, matching Lens client SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`, the
vanilla pack pinned by `assets/vanilla-source.json`, and the locally installed
1.26.50 iOS archive's `vanilla/materials/entity.material`. The implementation
is our own code from these references, not pasted reconstructed C++.

## Plane geometry and UV defaults


The former one-by-one default sampled a single texel-sized region of the
arrow's sixteen-by-five shaft and five-by-five cap. Runtime geometry now
uses the native face-size defaults for all entities, with explicit signed
`uv_size` remaining authoritative. No arrow dimensions are hardcoded in
production geometry.

## Two-sided material

The matched IPA defines `arrow:entity_alphatest`; that base inherits
`entity_nocull`, whose states disable culling. Both sides of a plane therefore
sample the authored face, with alpha testing. Our missing-face sentinel
previously discarded each reverse side even though the pipeline itself did
not cull it.

The current entity carrier does not retain render-controller material
selection. Pending a general material carrier, geometry logically bound to
`minecraft:arrow` receives this exact native two-sided contract. The lookup
checks every geometry binding, including alternate render-controller
geometries, rather than assuming a fixed geometry name or first binding.
Explicit separately authored reverse-face UVs remain untouched. This narrow
classification does not establish custom/server material parity.

## Actor rotation


Runtime retains the absolute yaw separately, keeps the arrow's body-root
yaw zero, and evaluates the pack's arrow animation. The authored body scale,
pitch, yaw and crossed-plane rotations remain pack data.

## Impact shake state


The pack's `arrow.entity.json` computes shake power from
`query.shake_time - query.frame_alpha`; the body animation consumes that
power. At actor-animation ticks the existing engine samples frame alpha
zero and interpolates bone poses for display. Exact native per-render-frame
reevaluation of this nonlinear expression remains incomplete.

## Verification and incomplete work

Focused render actor tests passed (43 passed; one benchmark ignored),
including fractional/default UV dimensions, explicit signed sizes, arrow
planes and two-sided sampling. The absolute-arrow-yaw query regression
passed. Six protocol status tests passed, including signed shake codec
roundtrips and rejection of every truncated packet-body prefix; the retained
shake countdown and query regressions also passed. Changes are local and
uncommitted. The owner manually tested and accepted arrow rendering on the
macOS/Metal client. This validates the reported appearance problem, not a
version-matched native flight/embedded-arrow comparison.

Incomplete: the general render-controller material carrier and exact
render-time frame-alpha query evaluation, including nonlinear impact shake.
Flight/embedding physics and trajectory prediction are not changed by this
rendering correction. No complete arrow or actor visual parity gate is closed.
