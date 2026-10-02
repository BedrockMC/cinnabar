# Native first-person item placement

This is our own implementation of the matched client behavior, not pasted reconstructed
C++ or Java Edition placement. Reference: `HashimTheArab/mcsrc-1.26.50`, revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current source view
`current/1.26.50.26/src/__unmapped/`. The matching Lens client executable has SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.

## Ordinary camera stack

RVA `0x04fa7e50` builds the shared placement from attack progress `a` and arm height `h`:

1. Swing translation: `(-0.4*sin(sqrt(a)*pi), 0.2*sin(2*sqrt(a)*pi), -0.2*sin(a*pi))`.
2. Camera anchor: `(0.56, -0.52 - 0.6*(1-h), -0.72)`.
3. Y rotation `45` degrees.
4. Attack Y/Z/X rotations: `-20*sin(a*a*pi)`, `-20*sin(sqrt(a)*pi)`,
   `-80*sin(sqrt(a)*pi)` degrees, in that order.

These ordinary items are already in camera space. Their poses must not inherit the
avatar's `rightItem` bone, body scale or third-person grip. Attachables retain their separate
skeleton path. The existing first-person projection is unchanged.

## Block branch and centered geometry

| Current RVA | Role |
| --- | --- |
| `0x06aab5f0` | Admits the ordinary cube shape in the block placement branch |
| `0x07042ce0`, `0x070430a0` | Presentation-mode-1 correction: Y rotation `-45` degrees |
| `0x06c407d0` | Selects a custom block display transform, otherwise the default |
| `0x07042580`, `0x07042290`, `0x07040520` | Constructs the default display matrix |
| `0x04f92b00`, `0x04f9a2e0` | Block flag skips the legacy sprite transform |
| `0x04fa6570`, `0x07069320`, `0x07066750` | Block mesh selection and native mesh centering |

The block branch skips the sprite's outer scale and legacy sprite transform. Its default
display parameters at VA `0x1502a1ac0` have presentation mode `1`, zero translation,
Euler rotation `(0,45,0)`, scale `(0.4,0.4,0.4)` and zero rotation/scale pivots. The correction
and default display rotations cancel, leaving the shared camera stack times uniform
scale `0.4`. There is no mirror or additional sprite offset.

The native mesh receives translation `(-0.5,-0.5,-0.5)` from VA `0x1502563e8`, centering
its `[0,1]^3` geometry. Cinnabar's cube already spans `[-0.5,0.5]^3`, so applying that
centering again would be incorrect. Third-person block placement and atlas selection
are not changed by this correction.

## Sprite branch

The sprite branch applies camera scale `0.4`, then its default legacy transform:
scale `1.5`, Y rotation `50` degrees, Z rotation `335` degrees, and local translation
`(0.075,-0.245,-0.1)`. Our sprite mesh's center correction and X mirror remain separate
from block geometry. The previously corrected ordinary sprite path is preserved.

## Verification and incomplete behavior

An isolated synthetic-atlas regression reproduced the old first-person block routing and
avatar-scale dependence before the fix. Tests cover camera-space routing, both sampled
item poses, avatar independence, atlas/mesh reuse, all centered cube corners, no X mirror,
swing/equip behavior and unchanged third-person placement. The same suite passes after
the correction.

The canonical debug executable is rebuilt. Focused equipment tests (30), workspace
all-target tests, formatting, strict Clippy and the architecture policy check pass locally.
The changes remain uncommitted. The corrected canonical executable has been restarted;
the owner subsequently reported that held blocks look right in their manual in-game test.

The modern held-block mesh config in RVA `0x07069320` disables the optional weighted-normal
face-light stage admitted by `0x070625a0`. Final mesh assembly at `0x0703c030` therefore
defaults that shade input to `1`; world-mesh directional factors cannot be borrowed for
this route. Downstream native material/shader lighting still needs its own witness.

The owner reports both ordinary sprite placement and the opaque held-block pose are working.
This is manual acceptance of those poses, not a complete version-matched frame comparison.
Grass's missing hand mesh and inventory icon are corrected. The compiler now resolves
authored carried faces once for both the inventory thumbnail and the held cube sheet;
the equipment runtime prefers these provenance-matched sheets to world-material faces.
The carrier remains compatible with the previous sprite-only format. See
[carried-texture source record](carried-block-textures.md). Live macOS/Metal frames show an
opaque green top, green side fringe and untinted brown soil in the hand, hotbar and open
inventory; the existing owner-accepted camera pose is unchanged.
Provisional, labeled incomplete: native sine-table rounding, non-cube block geometry,
custom block display transformations, exact held-block face lighting/material state,
item-specific legacy use/mirrored-art branches, custom render offsets and remaining attachable
variants. Modern bow routing now has its own
[source record](held-attachables.md). This correction does not close a complete first-person
vanilla parity gate.
