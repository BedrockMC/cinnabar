# Native first-person offhand placement

This records identified native behavior; it does not claim that every route below is
implemented or visually accepted. Reference repository revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`; current view
`current/1.26.50.26/src/__unmapped/04.cpp`. Constants were read from the matched Lens
Windows executable, SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`,
image base `0x140000000`. We implement these contracts in our own code, not by pasting
reconstructed C++.

## Separate render path

Current RVA `0x04f9e2e0` corresponds to the named 26.30
`ItemInHandRenderer::renderOffhandItem`, RVA `0x04320880`. It selects the cached offhand
stack at renderer offset `0xd0`, builds its render key using animation frame `-1`, pushes
the camera matrix, and draws the cached item through RVA `0x04f91850`. It does not
re-enter the ordinary main-hand `renderItem` transform after applying its camera pose.
There is no generic mirror-main-hand shortcut.

Maps, modern attachables, blocks, authored display transforms and legacy items have
distinct routes. The default legacy route is admitted by the item fallback or the
non-old-tessellation block display flag. Outside that route, the source selects the
authored world-matrix transformation instead.

## Default legacy sprite matrices

The following are post-multiplied onto the incoming camera matrix, in written order.
Angles below are degrees. These are native tessellator-frame matrices, not an instruction
to apply them directly to an unrelated centered mesh basis.

For `Item::isHandEquipped() == false`:

```text
Rx(90)
* T(-1.25, -1.125, 0)
* Rz(-80)
* Ry(-20)
* T(-0.3125, 0.25, -0.03125)
* S(1/16)
* S(16/max(icon_width, icon_height))
```

For `Item::isHandEquipped() == true`, unless the legacy Shield-blocking special case wins:

```text
T(-0.6875, -0.125, -1.53125)
* Ry(-10)
* Rx(70)
* Rz(80)
* S(1/16)
```

The hand-equipped predicate is not inferred from the identifier. Current Item vtable
slot `0x138` resolves to RVA `0x027d3390`, which reads bit 1 of Item offset `0x152`.
The named 26.30 `Item::isHandEquipped`, RVA `0x0a667720`, reads the corresponding bit
at its version-specific offset `0x112`.

Key current PE constants:

| VA | Value | Use |
| --- | --- | --- |
| `0x14feff2b4` | `pi/2` | Flat sprite X rotation |
| `0x1500de318` | `1.25` | Flat sprite negative X translation |
| `0x1501f3a80` | `-1.125` | Flat sprite Y translation |
| `0x1501f3a84`, `0x1501f3a88` | `-80`, `-20` degrees | Flat sprite Z/Y rotations |
| `0x15012a548`, `0x14ff1b150`, `0x1500de290` | `0.3125`, `0.25`, `0.03125` | Final flat sprite translation |
| `0x150114d6c`, `0x150121400`, `0x1501f3a9c` | `0.6875`, `-0.125`, `1.53125` | Hand-equipped translation |
| `0x1501f3aa0`, `0x1501f3aa4`, `0x1501f3aa8` | `-10`, `70`, `80` degrees | Hand-equipped rotations |
| `0x14ffa90e0` | `0.0625` | Native pixel-to-model scale |

Rotation constants in the executable are radians; the table shows their equivalent
degrees. In particular, `0x1501f3a9c` is **not** the ordinary block depth `0.72`.

## Geometry normalization

Current `_rebuildItem`, RVA `0x04f95030`, stores `16/max(width,height)` at cache-node
offset `0x2c0`. The returned render object begins at node offset `0x40`, so this is the
same field read at render-object offset `0x280` in the flat offhand branch. The named
26.30 reconstruction has the same relationship: node `0x298`, object `0x258`.
They are not two independent scaling fields.

The flat branch therefore normalizes the native pixel geometry to one model unit on
its longest side. Cinnabar's `held_sprite_vertices` already normalizes the longest
side and uses the held slab frame (X non-positive, Y non-negative, depth toward -Z).
Its X mirror and origin shift must not be repeated as if it were the centered
`extruded_sprite_vertices` mesh. Modern texture meshes retain that separate centered API.
Named 26.30 `TextureTessellator::tessellate`, RVA `0x044714f0`, emits positive column
X, depth Y and row Z in texel coordinates. For a UV-labelled held-slab point the exact
normalized native point is `(-held.x, -held.z, height/max - held.y)`. Thus the basis
conversion is `T(0,0,height/max) * Ry(180) * Rx(90)`. It is a proper rotation, not a
UV reflection. The flat branch already has longest-side normalization; the
hand-equipped branch additionally scales by `max/16`. Regression tests compare
front/back texel corners at square, rectangular and higher-resolution dimensions.

## Blocks and Shield legacy alternative

The block default branch starts at `T(-0.56,-0.52,-0.72)`, then selects the block's
display transform for presentation type `2`. The native default presentation array
(VA `0x1502a1b00`) has zero translation/pivots, Y rotation `-135` degrees and scale
`0.4`. Constructor RVA `0x07040520` negates Y/Z for type 2, yielding
`T(-0.56,-0.52,-0.72) * Ry(135) * S(0.4)`. Current
constants are `0x1501f3ab4 = 0.56`, `0x1501f3ab0 = -0.52`, and
`0x1501f3aac = 0.72`. Do not substitute the main-hand presentation type or sprite scale.

The legacy Shield-blocking route uses X/Z/Y rotations `2.5`, `177.5`, `-2` degrees,
then its offhand-height-dependent translation and scale `1.125` (VA `0x1501f3a30`).
Modern Shield attachables instead use their authored animation and owner binding; see
[held attachables](held-attachables.md) and [blocking state](shield-blocking.md).

## Offhand render context

Current `renderFirstPerson`, RVA `0x04fa7e50`, writes
`context.player_offhand_arm_height` separately from `variable.player_arm_height`.
The offhand value is
`previous_offhand_height + (current_offhand_height-previous_offhand_height)*frame_alpha`,
using renderer offsets `0x18c` and `0x188`. Main-hand heights use `0x184` and `0x180`.
The ordinary flat/hand-equipped offhand branches above do not apply the main-hand
attack-time swing stack. They also do not inherit the avatar's main-hand item bone.
There is no generic equip-height dip on these legacy branches: the only reads of
offhand height within RVA `0x04f9e2e0` are in its legacy Shield special case. The
outer first-person function calls offhand rendering before entering the main-hand
camera/equip stack (current source line 2627341), with a matrix push and the screen
aspect-layout adjustment but no offhand-height translation. Modern attachables use
the independently interpolated context through their authored first-person animations.

Native tick RVA `0x04f8c0f0` snapshots both hands independently, advances each toward
zero for a lowering transition or one otherwise, clamps each change to `[-0.4,0.4]`,
and replaces the cached stack at height `<=0.1` (or on an instant-update transition).
The PE constants are VAs `0x1500d39f0`, `0x14feff2a0` and `0x14ffab644` respectively.
The named 26.30 constructor RVA `0x04315730` initializes both current/previous
offhand heights to zero. Cinnabar now retains this independent clock across pose
resets and supplies its interpolated value to the shared attachable VM.

Incomplete gates: the complete map/legacy Shield route, authored display transforms,
and stack-specific native instant-update/equivalence predicates (the current retained
equipment feed supplies identifiers, not the full native cached ItemStack comparison).
The October 1 offline vanilla-BDS run on macOS/Metal at Retina scale 2 renders
the offhand Shield alongside main-hand blocks and the Crossbow, with independent
texture bindings and poses. The open survival inventory shows its offhand icon;
real take/place/restore gestures leave both that icon and the hand visible.
Geometry, clipping, layering, scale, colors and input ownership were inspected
in fresh rendered frames. This is functional/device rendering evidence, not a
matched native gallery or acceptance of the incomplete routes above.
