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
asynchronous custom artwork finished. Custom identities belong to the session
world registry and must be available independently of that artwork.

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
prediction freeze, timeout ordering, queue backpressure and production app wiring.
Custom identity regressions cover named states, missing visual resources and
incomplete definitions beside valid neighbors.

The live checks used macOS Metal on Apple M3 Pro, a 1280×752 logical window at
2× display scale, vanilla render mode and a debug build. Hive hub walking,
jumping and the compass game-selector menu worked. SkyWars transferred to the
destination and supported movement after the acknowledgement fix, but later
disconnected. A subsequent movement-only run ended with an "Unfair Advantage"
ban showing expiry `6d 23h`. Live connections stopped. That opaque server verdict does not
establish its cause; final live acceptance after the custom identity correction
remains blocked.

Persistent legacy-state upgrades, default-state reconciliation and unknown
property handling remain incomplete in `plan.md`. This work does not close the
broader terrain or server-pack visual parity gates. LoadingEnd currently follows
the local acknowledgement without the JSON-UI dimension-loading presentation and
navigation/resource lifecycle; that timing remains explicitly incomplete.
The local readiness delay currently uses a later app frame; the vanilla
readiness updater's exact scheduling clock remains unverified.
