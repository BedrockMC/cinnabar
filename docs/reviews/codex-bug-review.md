# Cinnabar bug review

Report-only review of `review/codex-perf`, based on `origin/dev-sonnet`, on
2026-09-29. No findings below were fixed as part of the performance pass.
Locations refer to the reviewed source; nearby performance changes may move them.

P1 means a remote input can abort the client or terminate an otherwise usable
session. P2 means incorrect session state, retained-resource growth, or visible
behavior under a concrete input sequence. Findings are based on code paths and
the stated reproductions, not a newly completed native Bedrock comparison.

## P1: JSON-UI expressions can overflow the process stack

- **Location:** `crates/json-ui/src/predicate.rs:437` and
  `crates/json-ui/src/predicate.rs:351`; ingress at
  `crates/json-ui/src/resolve.rs:210`.
- **Problem:** Parentheses recurse through `parse_atom -> parse_or`, and each
  `not` recurses through `parse_not`. Neither path bounds recursion or token
  count. The child-tree depth limit does not constrain expression strings.
- **Failure scenario:** A server UI child control sets `ignored` to 20,000 open
  parentheses, `true`, and 20,000 closing parentheses. This is a shallow JSON
  object with a 40,004-byte string, below the 16 MiB server UI budget at
  `app/src/runtime/network/resource_packs.rs:102`. Resolving that child can abort
  the process. `requires` and runtime binding expressions use the same parser.
- **Evidence:** A standalone `rustc` harness used the production predicate
  implementation, with only its `Value` and `Env` dependencies replaced by small
  stubs. Depth 64 exited **0**, returning `Some(Bool(true))`; depth 20,000 exited
  **-6 / SIGABRT**, with `fatal runtime error: stack overflow, aborting`.
  Temporary reproduction: `/private/tmp/cinnabar-predicate-repro.rs` and its
  executable of the same name without `.rs`. These temporary files are not
  committed. This proves the parser failure; it is not a full Bevy/live test.
- **Suggested fix:** Bound expression bytes, tokens, and recursive parse depth.
  Return an undecidable expression with a diagnostic at the bound. Test nested
  parentheses and repeated `not`, independently of JSON nesting limits.

## P1: Long acyclic UI inheritance chains bypass tree depth limits

- **Location:** `crates/json-ui/src/merge.rs:39`; call before tree resolution at
  `crates/json-ui/src/resolve.rs:65`.
- **Problem:** `flatten_def` checks inheritance cycles but recursively follows
  an arbitrarily long chain of distinct definitions. The `MAX_DEPTH` check at
  `crates/json-ui/src/resolve.rs:164` runs after inheritance flattening.
- **Failure scenario:** A server UI file supplies shallow sibling definitions
  `c0@c1`, `c1@c2`, and so on, and a displayed HUD/form control references `c0`.
  Ten thousand such definitions fit comfortably within the server UI byte
  budget and do not approach the JSON parser's nesting limit, but flattening
  them can abort the process.
- **Evidence:** A standalone `rustc` harness used production `merge.rs`, with
  minimal `Catalog`, `Value`, and `ControlRef` dependencies. A 64-definition
  chain exited **0**; a 10,000-definition chain exited **-6 / SIGABRT**, with
  `fatal runtime error: stack overflow, aborting`. Temporary reproduction:
  `/private/tmp/cinnabar-inheritance-repro.rs` and its executable of the same
  name without `.rs`. No reproducer or pack payload is committed.
- **Suggested fix:** Flatten inheritance iteratively or impose an explicit
  chain bound before recursion. Reject only the affected control, with a
  diagnostic. Test a long acyclic chain as well as a cycle.

## P1: Server sound weights overflow on the main thread

- **Location:** `app/src/audio/engine.rs:457`; server alternatives are admitted
  at `app/src/audio/server.rs:54` and weights at
  `app/src/audio/server.rs:43`.
- **Problem:** Every alternative is retained, while the aggregate weight is
  summed into `u32`. The definition count limit does not limit alternatives
  within one definition. Each alternative can have weight 65,535.
- **Failure scenario:** An admitted `sounds/sound_definitions.json` defines one
  sound with 65,538 copies of `{"name":"sounds/x","weight":65535}`. A
  `PlaySound` for that name reaches a sum of **4,295,032,830**, exceeding
  `u32::MAX` (**4,294,967,295**). Debug builds panic; ordinary release builds wrap
  and select from an incorrect distribution. This happens before PCM lookup,
  so the named sound file need not exist.
- **Evidence:** A compact JSON generator produced this definition in
  **2,293,884 bytes**, below `crates/resource-pack/src/lib.rs:34`'s 64 MiB file
  budget. The overflowing sum follows directly from the retained count and
  weights; no complete client run was performed for this finding.
- **Suggested fix:** Bound alternatives and aggregate retained definition
  resources. Use checked or wider weight arithmetic and skip an unusable
  definition without panicking. Test the aggregate boundary, including many
  high-weight alternatives.

## P1: SetScore removal encoding disagrees across the shipped Rust/Go pair

- **Location:**
  `crates/protocol/vendor/valentine/bedrock_versions/v1_26_44/src/types.rs:28414`,
  with a conflicting pre-validator at `crates/protocol/src/codec.rs:268`.
- **Problem:** The Rust owned decoder reads two optional-presence bytes for a
  removal objective name. The raw pre-validator reads one. The exact Go
  dependency pinned at `core/go.mod:53`,
  `github.com/hashimthearab/gophertunnel`
  `v1.25.3-0.20260908230935-3d9f4b7a4ac0`, uses one `OptionalFunc` in
  `minecraft/protocol/scoreboard.go`; its `minecraft/protocol/io.go`
  `OptionalFunc` writes one boolean followed by the value. This is a confirmed
  interoperability mismatch in the shipped dependency pair, without assuming
  which encoding a current native Bedrock client requires.
- **Failure scenario:** The core relays a removal with objective name `kills`.
  Its tail is `01 05 6b 69 6c 6c 73`. The raw pre-validator accepts that shape.
  The owned decoder treats `05` as a second presence byte, then `6b` as a string
  length of 107, and fails because only four bytes remain. Decode at
  `crates/protocol/src/login.rs:843` becomes a fatal session error at
  `crates/protocol/src/login.rs:310`.
- **Evidence:** Inspected the exact pinned Go source in the local module cache,
  its `OptionalFunc` implementation, the generated Rust decoder, and
  `crates/protocol/tests/ui_packets.rs:233`. That existing Rust test asserts two
  optional markers; it never compares a Go-produced frame with the Rust receive
  path. A Rust self-roundtrip therefore misses the mismatch.
- **Suggested fix:** Establish the version-matched native wire shape, align
  both pinned implementations and the pre-validator, and add a cross-language
  fixture that enters the actual play receive path.

## P2: Objective-less score removals never remove the retained row

- **Location:** `crates/protocol/src/ui.rs:524` and
  `crates/ui/src/scoreboard.rs:550`.
- **Problem:** An absent removal objective is converted to an empty string.
  The scoreboard store then requires an objective with that exact name before
  processing the score ID. Absence cannot be distinguished from a present
  empty name, and the retained ID is never searched.
- **Failure scenario:** Objective `kills` contains score ID 7. A valid removal
  with an absent objective marker and ID 7 is decoded, normalized to objective
  `""`, and discarded as a missing objective. The old row remains. In a mixed
  packet, this rejection also discards all other staged changes in the batch.
- **Evidence:** The exact pinned Go `ScoreboardEntry.Marshal` permits an absent
  objective, and the existing Rust optional-marker test covers decoding that
  absence, but does not apply it to a populated scoreboard.
- **Suggested fix:** Preserve objective-name presence and resolve an
  objective-less removal by retained scoreboard identity. Confirm the native
  identity/removal contract, and test that an absent objective removes a
  previously displayed row without dropping unrelated entries.

## P2: A cancelled session can replace the current server sound pack

- **Location:** `app/src/runtime/network/resource_packs.rs:74` and
  `app/src/audio/server.rs:172`; asynchronous teardown at
  `app/src/runtime/network/session.rs:490`.
- **Problem:** Pack preparation publishes sounds into a process-wide mailbox
  before the Bootstrap session generation is checked. The mailbox generation
  counts completion order; it does not identify the owning network session.
  Sound preparation happens outside the mailbox lock. Synchronous preparation
  at `app/src/runtime/network/session.rs:589` does not observe cancellation.
- **Failure scenario:** Session A is cancelled while preparing a large sound
  pack. Session B joins and publishes its sound pack. A finishes later and
  overwrites the mailbox. A's Bootstrap is correctly rejected at
  `app/src/runtime/network.rs:301`, but `AudioEngine::poll_server` still installs
  A's sounds in B, or installs them after returning to the menu.
- **Evidence:** Static concurrency review of cancellation, deferred worker
  reaping, mailbox publication, and Bootstrap admission. No timing-sensitive
  live reproduction was performed.
- **Suggested fix:** Return the prepared sound pack in the generation-owned
  pack result and install it only after accepting that Bootstrap. Test delayed
  completion of A after B with controlled barriers.

## P2: Deferred audio starts and compressed decode jobs lack aggregate limits

- **Location:** `app/src/audio/engine.rs:511` and
  `app/src/audio/bank.rs:77`.
- **Problem:** `MAX_QUEUED` limits one frame's incoming requests, and voice
  limits are checked only after PCM is ready. Each pending lookup creates
  another `PendingStart`; an already-decoding path returns `Pending` at
  `app/src/audio/bank.rs:230`. Streamed starts survive indefinitely while that
  decode remains outstanding (`app/src/audio/engine.rs:324`). Decode jobs use
  an unbounded channel containing owned compressed bytes, with two workers.
- **Failure scenario:** Repeated sound events for a slow queued streamed track
  add pending starts every frame while earlier decodes occupy both workers.
  Many distinct sound names additionally retain the compressed input of every
  queued job. A frame queue capped at 256 and a voice pool capped at 48 do not
  cap either backlog. Slow decoding can therefore create a large memory and
  replay burst, despite those apparent limits.
- **Evidence:** Static review of input admission, pending-start lifetime,
  in-flight lookup, and worker queues. No fixed maximum pending count or queued
  compressed-byte budget exists in those paths.
- **Suggested fix:** Bound pending starts and aggregate queued decode bytes,
  coalesce suitable duplicate requests, and reject/drop excess work with
  counters. Test a deliberately blocked decoder under sustained event ingress.

## P2: A zero-rate server WAV creates a voice that never finishes

- **Location:** `app/src/audio/server.rs:100` and
  `app/src/audio/voice.rs:157`.
- **Problem:** The WAV decoder accepts PCM16 with one or two channels but does
  not validate the sample rate. A zero-rate PCM produces a zero resampling
  step, so its position never reaches the end.
- **Failure scenario:** A server sound override contains a WAV with rate 0 and
  a nonempty PCM data chunk. Playing it as a one-shot repeatedly emits its
  first sample and never marks the voice finished. It can produce permanent DC
  output and consume per-sound/voice slots until explicit cancellation.
- **Evidence:** Static data-flow proof: accepted `rate` is stored unchanged in
  `Pcm`; `rate * pitch / OUTPUT_RATE` is zero; the one-shot completion check at
  `app/src/audio/voice.rs:130` depends on the advancing position. The FSB/Ogg
  paths already reject unsupported rates, unlike this WAV path.
- **Suggested fix:** Validate WAV rate and complete frame alignment before
  constructing PCM. Test a zero-rate WAV and assert it is skipped.

## P2: Main audio ingestion does not enforce the dimension epoch

- **Location:** `app/src/audio/systems.rs:156`.
- **Problem:** The main audio engine validates session, dimension number, and
  sequence, but ignores `SequencedAudioEvent::dimension_epoch`. It also clears
  voices and record state on session changes only. The older narrow named
  audio path explicitly checks the epoch at `app/src/named_audio.rs:102`.
- **Failure scenario:** One stream poll commits an old dimension-0 sound,
  changes to dimension 1, and returns to dimension 0 before audio ingestion.
  The queued old sound has the current dimension number and a fresh sequence,
  so it is played in the new dimension-0 visit. The epoch changes at
  `crates/client-world/src/stream/sequencing.rs:475`, and each audio event
  retains its original epoch at that file's line 617, but ingestion ignores it.
- **Evidence:** Static review of committed-audio retention, dimension changes,
  event forwarding, and both audio consumers. No native audio comparison was
  performed.
- **Suggested fix:** Compare event epochs and bind audio state to the current
  epoch. Cancel or reset dimension-owned state when it changes. Test a
  dimension roundtrip with an old sound still queued.

## P2: Jukebox bookkeeping grows for the entire session

- **Location:** `app/src/audio/systems.rs:204`.
- **Problem:** `IngestState::records` inserts a sound name for each jukebox
  coordinate. Entries are removed only by another record event at that exact
  coordinate or by a session reset. Finishing a track, unloading its chunk,
  changing dimension, and `StopSound` do not remove the record entry. There is
  no count limit.
- **Failure scenario:** A long session receives record-start events at many
  distinct coordinates, with a valid disc item ID and no later stop event for
  those unloaded locations. Audio voices finish or are rejected by their limit,
  but every coordinate remains in the map, allowing retained memory to grow
  throughout the session.
- **Evidence:** Inspected all `state.records` insert/remove/clear uses in
  `app/src/audio/systems.rs`. The insertion precedes voice admission, so even
  inaudible or rejected starts retain entries.
- **Suggested fix:** Track actual voice lifetimes and clear dimension-owned
  entries on epoch changes. Add a bounded retention policy for remote record
  coordinates and test many rejected/out-of-range start events.

## P2: Changing server actor artwork suppresses all neutral texture pages

- **Location:** `crates/render/src/actor_render/artwork.rs:32`,
  `crates/render/src/actor_render.rs:232`, and
  `crates/render/src/actor_render.rs:852`.
- **Problem:** GPU artwork refuses every identity change after its first
  upload. Session pack application intentionally changes that identity:
  `app/src/runtime/network/actor_publication.rs:96` applies pack textures, and
  `crates/render/src/actor/artwork.rs:336` hashes the appended pixels into the
  identity. A refused replacement sets `artwork_current` to false; drawing then
  skips every span whose texture page is not the player-skin page 0.
- **Failure scenario:** After a server's neutral artwork has reached the GPU,
  transfer to another server with different entity textures. The new scene
  carries the new pack artwork, but the renderer keeps the old GPU generation
  and suppresses mobs and other neutral-page layers. Player bodies using page
  0 continue to draw. Returning to the exact first artwork identity can recover;
  ordinary pack replacement cannot.
- **Evidence:** Static review of pack pixel hashing, scene publication, GPU
  admission, and the draw-span skip. No live transfer/render capture was made.
- **Suggested fix:** Allow a bounded artwork-generation replacement and retain
  the old GPU resources until submitted draws no longer use them. Test a
  base-to-pack and pack-to-pack transition through actual render preparation.

## Review scope and limits

The review inspected Rust play packet routing and normalization, generated
packet decoding, blob-cache recovery, world/chunk admission, actor retention,
JSON-UI parsing/resolution/binding, particle limits, and audio ownership/queues.
It also inspected Go relay/transfer handling, resource-pack preparation/cache,
local-world process management, and control framing. Apparent issues that were
already protected by checked arithmetic, semantic-skip routing, or bounded
retention were excluded. Item resolution, inventory handling, and equipment
presentation were not changed.

The two stack-overflow experiments used standalone temporary harnesses. No
Cargo command, Go test, live server join, or target-platform acceptance run was
performed by the report reviewer. Workspace verification is recorded separately
by the agent responsible for the performance commits.
