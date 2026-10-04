# Inventory hover tooltip

The inventory's `common.hover_text` custom control is rendered by retained JSON-UI
nodes. It does not introduce a second UI renderer or embed Mojang artwork.

The name line shares its resolution and native custom-name formatting with the
selected-item HUD; see [Stack display names](item-display-names.md).

## Matched sources

The reference checkout is `HashimTheArab/mcsrc-1.26.50`, revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`.

- Current `current/1.26.50.26/src/__unmapped/05.cpp`: constructor `05e10e10`,
  clone `05e10f10`, update `05e11010`, render `05e12ba0`.
- Current `0f.cpp`: the constructor's `0f91aa80` test accepts the integer
  JSON variant only before `0f91a760` reads `hover_text_max_width`.
- Current `04.cpp`: `04c18ff0` selects the bitmap font's default scale (one),
  `04c19060` returns its wrap pitch, default scale multiplied by ten, and
  `04c2f5e0` (`Font::getLineLength`) rounds the widest measured line upward.
- Named older `reference/26.30/src/by-owner/h/HoverTextRenderer.cpp` establishes
  those functions' identities. `BitmapFont.cpp` identifies `getScaleFactor`
  and `getWrapHeight`; `Font.cpp` identifies the `drawCached` signature. The
  older build is not used to invent current constants.
- The pinned pack's `ui/ui_common.json`, `hover_text`, defines max-width zero
  (no wrapping), custom renderer `hover_text_renderer`, layer five and
  `allow_clipping: false`. The texture path is native renderer-owned,
  `textures/ui/purpleBorder`; its pinned JSON sidecar defines a 16×16 source
  with four-pixel nine-slice insets. Runtime server overrides still win.

Numeric constants were read from the matching Lens
`client-1.26.50.26/Minecraft.Windows.exe` (SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`):

| Address | Value | Use |
| --- | --- | --- |
| `14ffab650` / `14ffab654` | +10 / −10 | Mouse offset, bitmap wrap pitch |
| `14ff9cdc4` | 8 | Added box width and height |
| `14fea4060` | 1 | Added measured width and default font scale |
| `14fee6588` / `14fec3380` | −1 / 0.5 | First-line minimum-height calculation |
| `14ff666f0` | 5 | Text inset, both axes |
| `14feff2e0` | −0.5 | Oversized-box horizontal centering |
| `1500567d0` | 60 | Touch-mode bottom reservation, not mouse |

## Implemented mouse path

The text is measured in virtual UI pixels, keeping every newline. Native bitmap
pitch is ten; this is intentionally distinct from the general label/chat pitch.
The native first-line height rule is
`max(((fontScale−1)×0.5+1)×8, wrapHeight×fontScale)`, with one further
`wrapHeight×fontScale` per newline. Default bitmap scale one and wrap height ten
select ten over the minimum eight. The accepted open font has eight-design-pixel
ink and uses these default native metrics; its glyph bounds do not replace the
native tooltip-box height rule.
The background width is ceil(text width) + 1 + 8, and height is truncated text
height + 8. Text starts at box origin + (5,5), white with inherited opacity and
format codes. The native call disables shadow, so the tooltip does not inherit
the HUD's drop shadow.

The initial mouse offset is (+10,−10). Bottom overflow subtracts the excess from
the vertical offset. Right overflow flips the entire box left of the pointer;
it does not clamp against the right edge. If this also crosses the left edge,
the box centers above the pointer. No extra top-edge clamp is introduced. Its
clip is the full content viewport, not the hovered slot. A positive authored
integer `hover_text_max_width` requests wrapping; zero or a fractional JSON
number leaves it unrestricted, as in the native constructor.

The background is nine sliced from the runtime texture and its sidecar, preserving
the native border and fill rather than reconstructing colors. The JSON-UI texture
residency pass now includes this custom renderer's resource. On-demand vanilla
texture fallback reads its JSON sidecar as well as its bitmap. Tests use synthetic
art and retain no licensed pixel data.

The open font remains the repository's accepted deviation: glyph widths differ
from the Mojang bitmap font, but geometry and pitch are applied to the selected
font's measured widths. Touch/gamepad cursor acquisition and touch-only bottom
reservation are not claimed complete by this mouse-path fix.

## Verification

Focused tests cover native placement, overflow, integer box sizing, preserving
blank lines, requested wrapping, passing the native width property through the
JSON-UI emitter, actual background residency/nine-slice output, alpha propagation,
fullscreen clipping and absence of a shadow. The root agent runs builds and live
validation; this document alone does not close that gate.

The tooltip text bridge also preserves all colors from the UI engine's existing
Bedrock formatting palette, rather than retaining only gray enchantments and
purple lore. Each independent line resets formatting before its base color, while
server-authored codes within the line still override that base. An item's already
resolved component name-color code is preserved directly; no rarity rule changes
are introduced. The existing component RGB table disagrees with the UI palette for
some material colors, so those names use their native code rather than guessing a
nearest RGB. Arbitrary non-palette line RGB is not representable by Bedrock format
codes and remains an explicitly incomplete input case; current tooltip line
producers use the native name code, gray enchantments or purple lore.

## Live mouse acceptance

The same canonical macOS/Metal Retina-2 build recorded in
[player-preview rendering](player-preview-rendering.md#live-follow-up-acceptance)
was checked against the existing offline official BDS session. Fresh, unresampled
2560×1440 framebuffer captures were inspected:

- `2026-10-02_03.22.44.png`: Grass Block uses the resident native purpleBorder
  nine-slice, a dark fill, the source-derived mouse offset and text inset, and no
  inherited HUD text shadow.
- `2026-10-02_03.23.38.png`: after BDS confirmed Sharpness III enchanting, Diamond
  Sword and its gray enchantment line share the source-derived tooltip pitch and
  expanded box height. The box can extend past inventory control bounds without
  slot/container clipping, and is drawn above the slot icons.

Text legibility, border/fill colors, geometry, layer/clip behavior and actual hover
input passed the rendered-frame check. Synthetic regressions cover edge overflow,
blank lines, opacity, authored wrapping and nine-slice sizing. The accepted open
font and unimplemented touch/gamepad modes remain the limitations above; this is
not a version-matched native screenshot comparison.

The upstream-integrated rerun uses the executable hashes and offline BDS session
recorded in [the preview rerun](player-preview-rendering.md#upstream-integrated-rerun).
`2026-10-02_04.09.46.png` verifies Grass Block's single-line native border/fill,
inset and mouse placement. `2026-10-02_04.10.41.png` verifies Diamond Sword plus
the server-confirmed Sharpness III line, gray formatting, native pitch and a box
extending below the inventory without container clipping. Both are fresh
2560×1440 F2 frames from the latest canonical build, inspected after the upstream
gamma-space UI compositing changes were integrated. The font and non-mouse
limitations remain unchanged.
