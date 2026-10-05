# Hive connection and dimension transfer

An authenticated Hive hub join exposed failures in pack admission, terrain
decoding and dimension transfer. All 24
required packs carried display labels as selected subpack names, although their
manifests declared no subpacks. Admission rejected the entire required stack.
After admitting root resources, persistent block palettes decoded as air. After
resolving those palettes, terrain appeared 64 blocks below the server's actors
and player because the named dimension definition was ignored.

The hub then supported walking, jumping and the server's game-selector menu.
Selecting SkyWars changed dimension to a staging position at Y 4000. Prediction
continued falling and no dimension acknowledgement was sent; Hive disconnected
with "You didn't finish joining." The destination handshake now pauses prediction
and retains the loading-screen identifier.

Captured persistent identities also exposed missing custom floor blocks. The
session used sequential wire IDs, so the custom overlay omitted its canonical
hash table. Persistent entries therefore could not resolve advertised blocks
such as `hive:cream_brick` and `hive:stone_herring_bone_bricks`; their existing
solid collision shapes never reached the world. Custom overlays now retain
complete identities in either wire mode, preserving visual and collision slots
when an individual definition cannot provide a complete identity.

Offline replay additionally exposed early decode snapshots taken before the
asynchronous custom artwork finished. The session world registry now installs
custom identities before terrain admission; immutable worker snapshots retain
them independently of artwork publication.

The captured definition named `minecraft:overworld`, with minimum Y 0, height
256 and numeric dimension type 3. StartGame and LevelChunk used dimension 0.
The definition arrived before StartGame and remained ahead of chunks through
the login handoff. Pack archives, packet captures and rendered frames remain
outside git.

| Vanilla rule | Implementation |
| --- | --- |
| An unavailable selected subpack falls back to root resources. | Validate the root manifest before selecting a declared folder; retain the wire label as metadata. |
| Persistent palettes identify blocks by name and typed states. | Decode network NBT, qualify vanilla short names and resolve the canonical identity in the active registry. Unknown entries use air. |
| Persistent custom identities are independent of the session's wire ID mode. | Keep the custom hash lookup in sequential overlays too; incomplete identities skip only their own slots. |
| Advertised block identity is available before resource artwork. | Install canonical identities in the session world registry before terrain decoding and retain them in immutable worker snapshots. |
| Legacy cube textures come from `blocks.json` when explicit visual components are absent. | Apply scalar, three-face or six-face bindings in stack order before caching the effective visual. |
| A custom block without a collision component retains a full-block shape; disabled collision is empty. | Resolve the advertised block before applying its existing collision policy. |
| Named builtin definitions override their dimension's default bounds. | Retain the definition name; the overworld key selects dimension 0 independently of the definition's numeric type. |
| Definitions retain their first registration; instantiated dimensions retain their height. | Admit definitions before later decode snapshots and freeze a dimension's effective range when terrain first uses it. |
| Terrain consumers share one dimension range. | Decoding, requests, residency, collision queries, block entities and sky ceilings read the session owner. |
| Dimension acknowledgement belongs to the session, including sentinel actor IDs. | Admit server action 14 without filtering its actor ID. |
| Transfer completion waits for the server acknowledgement and a loaded destination area. | Wait for the acknowledgement, or a timeout strictly over ten seconds followed by another tick, and check inclusive position ±16 bounds. |
| Readiness follows the current player position and skips sections outside the dimension height. | Accepted server teleports update the anchor; authoritative air columns count as present. Out-of-range Y uses the dimension's loading fallback (0, or 50 in the End). |
| Loading notifications retain the same optional identifier. | Queue LoadingStart, the local action-14 acknowledgement, and LoadingEnd in order; retry only writes not already admitted. |

Regression coverage includes required pack stacks, NBT stream boundaries,
every pinned vanilla typed state, both runtime ID modes, early login definitions,
inline placement, request origins, pending decode ordering and invalid ranges.
Transfer regressions cover sentinel acknowledgements, metadata and packet
roundtrips, readiness boundaries, authoritative air, current-position changes,
prediction holds with continuous stationary input ticks, timeout ordering,
queue backpressure and production app wiring.
Custom identity regressions cover named states, missing visual resources,
incomplete definitions beside valid neighbors, admitted ranges, offset overflow,
asset precedence and snapshots retained across registry replacement.
Visual regressions cover legacy face bindings, per-block cache identity, valid
lower bindings beneath malformed overrides, geometry-specialized animation
instances, large artwork page sets and immutable geometry aliases.

The live checks used macOS Metal on Apple M3 Pro, a 1280×752 logical window at
2× display scale, vanilla render mode and a debug build. Hive hub walking,
jumping and the compass game-selector menu worked. SkyWars transferred to the
destination and supported movement after the acknowledgement fix, but later
disconnected. A subsequent movement-only run ended with an "Unfair Advantage"
ban showing expiry `6d 23h`. Live connections stopped. That opaque server verdict
does not establish its cause; final live acceptance after the custom identity
correction was blocked until the user reported the account unbanned.

The final offline run used build `8d32996a`, eight captured terrain columns, one
synthetic air neighbor and a local teleport anchor. All 1,581 advertised custom
states were registered before artwork with zero skipped definitions. Walking
reached the captured `hive:cream_brick` at [-11, 39, 0]; the eye rested at Y
41.62001, jumped to 42.87221 and returned to the same floor. Fresh rendered frames
showed the restored custom surfaces and working input. The local bridge omitted
encrypted server artwork and ignored gameplay requests: diagnostic textures were
expected, and this run establishes neither server acceptance nor visual or
performance parity. Its complete replay report ended on the local client's exit.

Persistent legacy-state upgrades, default-state reconciliation and unknown
property handling remain incomplete in `plan.md`. This work does not close the
broader terrain or server-pack visual parity gates. The local readiness delay
currently uses a later app frame; the vanilla readiness updater's exact scheduling
clock remains unverified.

On 2026-10-05, current dev was integrated after the user reported the account
unbanned. Transfer ownership was consolidated into the app coordinator, retaining
the server acknowledgement and loaded-area gates. Input ticks continue while
prediction is held; LoadingEnd now waits for the JSON-UI loading presentation and
a fresh destination frame. Three reproduced regressions cover named overworld
probe heights, raised custom air columns and synced-block range freezing.
The renewed hub join admitted all 24 packs and registered all 1,581 custom states,
but live frames showed diagnostic custom terrain and missing custom actors. Many
blocks use legacy `blocks.json` texture bindings that the visual compiler omitted;
one cached diagnostic visual concealed hundreds of missing bindings. The entity
bundle separately failed the compiled animation-table bound. After correcting
that bound, the captured stack compiled 1,014 artwork bindings and 1,777 textures
without dropping source files. Publication initially rejected 636 bindings at the
artwork page ceiling and rejected the combined vanilla/server vertex catalog.
CPU artwork page identifiers now retain all 551 pages with zero rejected bindings.
Exact immutable vertex payloads share storage while keeping their independent rig
metadata and routes; the combined catalog publishes 548,922 vertices within its
existing bound. The captured-stack publication regression passes, all 25
block-overlay checks pass with the encrypted fixture, and all 585 renderer checks
pass. These offline checks retain original artwork and geometry.

The following live Metal run restored custom floors and NPC models. User frames
still exposed opaque title backs, black hologram panels, floating sheep, dark
flowers, incomplete climbing vines and lamps, and diagnostic hay. The hub stayed
connected for about 23 minutes before an opaque server Disconnect message ended
the session. Its cause is unresolved. Subsequent client startup found the core
endpoint absent; no new successful join is claimed from that attempt.

Offline inspection reproduced a 32-face truncation on the 33-face climbing vine
and 40-face large lamp, omitted legacy light filters and authored material states,
rejected 41-choice geometry selectors, and stale absolute registry-ID checks for
hay. Focused regressions now pass for those fixes, all 41 captured hologram
variants, authored GPU material states and targeted-entity F3 diagnostics.
Lantern bodies, caps and crossed handles replace the collision-box fallback;
their sprite sampling survives carrier publication. Geometry measurements use a
near-version witness and remain fallback support pending exact-version evidence.
NPC controller particle bindings and alpha-first hex tint decoding also pass
their owner and app route regressions. The following inspection build joined Hive
with no missing textures, missing geometry, truncated models or unevaluated
permutations in its block overlay. User frames show readable game titles, restored
plants and visible NPC particles, while title backgrounds remain too pale.
The user identified the floating-text hosts as sheep with server scale zero;
the client incorrectly replaces that scale with one. Its correction is pending.
This inspection session ended after about nine minutes with another opaque
server kick and zero decode errors. Reconnection is not session acceptance.
Full visual acceptance and the unresolved disconnect remain open in `plan.md`.
