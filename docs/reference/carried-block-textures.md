# Native carried block textures

Reference: `HashimTheArab/mcsrc-1.26.50`, revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current view
`current/1.26.50.26/src/`. Matching Lens executable SHA-256:
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.
This is our own implementation from identified native contracts, not copied C++.

## Identified contracts

- `__recovered/TextureJSONParser.cpp:560–579`, RVA `0x01a2ce10`: hexadecimal
  overlay colors keep the low 24 RGB bits, normalize by 255 and force alpha to one.
- `__unmapped/08.cpp:212905–212914`, RVA `0x08119570`: texture-atlas overlay uses
  source alpha as a color mask. For normalized source color `C`, source alpha `a`
  and overlay `T`, `mask = a*T.a` and `RGB = C*(1-mask) + T.rgb*C*mask`.
  Positive overlay alpha makes the resulting face opaque.
- `08.cpp:212921–212927`: normalized output is multiplied by 255 and truncated.
- `06.cpp:838952–838961`, `839487–839488`, RVAs `0x0648e1a0` / `0x064c4fa0`:
  parsed overlay reaches tile offset `0x30`; atlas mip construction passes that
  value as overlay argument 15 (`06.cpp:878087`, `878509–878513`).
- `__recovered/TextureAtlas.cpp:39–46`, RVA `0x08119420`: sampling retains RGBA order.
- Pinned pack `blocks.json:2748–2753` declares grass's carried faces;
  `textures/terrain_texture.json:61–70` selects a precolored grass top, dirt bottom
  and the grass side with an authored overlay color. No biome tint is borrowed.

## Correction and verification

Previously both paths rejected the side's overlay metadata: inventory fallback produced
no icon and held-sheet construction refused tinted world materials. The compiler now
retains reviewed carried overlay metadata and resolves six colored tiles once. Those
tiles feed both the thumbnail and an opaque six-face sheet. A bounded, validated carrier
extension binds sheets to block visuals; legacy sprite-only carriers remain readable.
Runtime sheet admission checks manifest identity and the entity carrier's visual bounds.
Unknown metadata, malformed colors, animations and unsupported geometry remain refused.

The compiler's missing-grass regression failed before correction, as did the runtime
carried-sheet admission regression. Five compiler carried regressions, six icon-carrier
tests, 46 pack-parser tests, 30 equipment tests and pinned-pack grass coverage pass.
Workspace all-target tests, formatting, strict Clippy and the architecture policy pass.
The rebuilt local icon report has 14 carried-sheet bindings and no unresolved grass item.

Live macOS 26.3 / M3 Pro / Metal verification used the canonical optimized-debug executable,
logical window 1280x752, rendered content 2560x1440, Retina scale 2. Ignored frames
`.local/screenshots/2026-10-01_12.08.15.png` and `2026-10-01_12.09.02.png` show the
grass hand/hotbar and open inventory: opaque green top/fringe, brown untinted soil,
legible item label, correct slot clipping, and unchanged held-cube camera pose.
Native window capture also verified the open inventory; local macOS input was the explicit
fallback after freshly discovered native input failed its focus guard.

Changes remain local and uncommitted. Inventory-thumbnail shading is still provisional,
and complete held material/lighting and custom geometry/display parity are incomplete.
The two reported bugs are functionally live-tested; no broader vanilla parity gate is closed.
