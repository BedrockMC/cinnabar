# Resource-pack continuation references

`R:` paths are relative to the read-only 26.30 reconstruction's `src/by-owner/`.
`V:` paths are relative to the pinned vanilla 1.26.50 resource pack.
Lens entries below are source-backed canonical reads from client artifact 6,
version 1.26.50.26. No reconstructed source is copied into Cinnabar.

## Device tiers and library settings

Lens `FUN_140308da0`, RVA `0x308da0`, uses strict greater-than comparisons with
2, 4, 6, 8 and 12 GiB of physical memory. The resulting tier is 0 through 5.
`R:h/HardwareMemoryTierUtilImpl.cpp:10` corroborates every boundary.
`R:r/ResourcePackStack.cpp:450` selects the highest supported subpack; equal
tiers replace the candidate, so the last declared tie wins. Root resources
remain the fallback if no declared subpack is supported.
`V:ui/pack_settings_screen.json:89` binds the selected name; `:93` binds support;
`:143` and `:146` bind slider value and steps; `:174` shows the unsupported warning.

Pack thumbnails use `V:ui/resource_packs_screen.json:41` and `:49` (`#icon_path`).
Lens `0x25a1270` and `0x1b49550` resolve pack icon paths; `0x1b93fe0` registers
`pack_icon.png` and `bug_pack_icon.png`. These reads do not yet establish the
retail missing-icon fallback. Cinnabar currently uses the pinned base pack icon;
that fallback remains a provisional deviation.

Immutable local import revisions and live Apply are Cinnabar extensions. Import
never overwrites an applied archive. An acknowledgement persists the exact
revision it published; a newer staged selection remains intact. Old revisions
are pruned only when the acknowledged selection is still the latest selection.

## Fonts and item routes

`R:b/BitmapFont.cpp:1449` and `:1664` scan `ASCII_CHAR_INDICES`; the 256-cell table
was read through Lens at reference address `0x10db2d21c` in the earlier handoff.
It includes extended characters and differs from Latin-1 and CP437.
`R:f/FontLoadingUtils.cpp:44` reads `ascii_font_file` and `unicode_file_prefix`;
`:477` reads `font_file`, and `:490` reads `target_font_render_size`.
`R:t/TrueTypeFont.cpp:175`–`:176` tries TTF and OTF extensions.
Lens `0x177e1f0` reads version-one `font/font_metadata.json`, `font_format`,
`font_name`, and dispatches bitmap, TrueType and TrueTypeMSDF constructors.
Lens `0x4c3df00` reads `font_language_code` for alias selectors.
`V:ui/enchanting_screen.json:129` selects `font_type: rune`; the pinned pack's
`texts/ja_JP/font/glyph_*.png` supplies language-scoped bitmap sheets.

Runtime named-font routing now reaches both JSON-UI measurement and painting.
Outline fonts currently use the existing alpha rasterizer. Exact TTFMSDF and
precomputed MSDF shader/data parity remains incomplete; precomputed MSDF is not
silently treated as a regular bitmap. Alias ordering and outline metrics still
need a native version-matched comparison before closing their parity gates.

Item replacement follows the pinned carrier's `ItemVisualKey.metadata`, source
path and `ItemTextureReference.variant`. Atlas arrays retain their original
indices when a member is unreadable. Aliases sharing the same carrier route can
redirect its texture through an overriding `textures/item_texture.json`.
Context-dependent icon selection is not established by these static metadata
routes. Lens `0x7fcc370`, `0x5056ee0` and `0x9c82630` expose the separate
`#should_show_bundle_open_front` path; that contract remains open.

## Publication and invalidation

Live reload is an extension, not a claimed vanilla live-world feature. Candidate
world meshes, biome records, GPU arenas and transparent draw references are
prepared before the atlas transaction is acknowledged. Publication precedes
render queue construction. A failed candidate retains the current GPU set.

Subscribers record actual successful and missing reads, plus directory names.
Cache entries retain this evidence; an untracked cache cannot masquerade as a
complete dependency snapshot. Layer order remains part of each fingerprint.
This is subscriber-level invalidation, not individual GPU texture patching.

The mailbox attaches the immutable geometry and atlas in one lock. Repeated
requests preserve the acknowledged snapshot. Staged transparent references cover
all residents, so moving the camera during preparation cannot expose an unstaged
chunk. A live biome revision change rejects the candidate for a fresh Apply.
Queued world removals survive resource publication. Non-block upload paths and
native frame continuity remain unverified.

## Reconciliation verification

The reported three app harness failures required `PackReload` although those
minimal worlds do not install Global Resources. Network event handling now accepts
its absence; production still installs it. The render source-order regression now
finds the completed texture pair in the extracted builder. Test targets compile;
these statements do not claim the tests executed successfully.

Limiter-controlled app and render test-target checks pass. The existing local
architecture checker passes. The full authorized remote gate cannot currently
start because its SSH proxy targets a failed server pod. Native measurement also
needs a release executable; this task has requested an exception to the explicit
small-local-check-only rule and has not assumed permission. No native metrics or
new screenshots are claimed.
