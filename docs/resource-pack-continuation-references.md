# Resource-pack continuation references


## Device tiers and library settings


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
