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



## Blocks and Shield legacy alternative


The legacy Shield-blocking route uses X/Z/Y rotations `2.5`, `177.5`, `-2` degrees,
then its offhand-height-dependent translation and scale `1.125` (VA `0x1501f3a30`).
Modern Shield attachables instead use their authored animation and owner binding; see
[held attachables](held-attachables.md) and [blocking state](shield-blocking.md).

## Offhand render context



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
