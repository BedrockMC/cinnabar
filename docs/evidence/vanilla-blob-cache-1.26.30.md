# Vanilla blob-cache and chunk-ordering reference for Bedrock 1.26.30

## Evidence boundary and provenance

This document records authoritative vanilla behavior observed in the Minecraft
Bedrock 1.26.30 client binary with its debug symbols. The observations were
contributed by the repository owner. Symbol names and relative virtual addresses
(RVAs) are included only so that a future investigator can re-verify the
observations. No disassembly, proprietary source, or copied code structure is
reproduced here.


## World-change ordering uses receive-side pause bookkeeping

Vanilla buffers selected world-change packets by pausing a per-connection
receive-side bucket. It does not use a per-column apply barrier.


- `UpdateBlockPacket`
- `UpdateBlockSyncedPacket`
- `UpdateSubChunkBlocksPacket`
- `BlockActorDataPacket`
- `BlockEventPacket`
- `ContainerOpenPacket`
- Reserved packet 109

This list is exhaustive. The repository owner established it by scanning all of
the client's `.text` section for `call [reg+0x7f0]`, finding 31 call sites, and
resolving every hit.


When the count for the packet's column is non-zero,
`queueHandleWorldChangePacket` calls
`NetworkSystem::setConnectionChannelPaused(id, 0, true)` to pause receive-side
bucket 0 and stashes a
`std::function<void(BlockSource&)>` in `mConnectionPausedCallbacks`. The
corresponding client log string is
`"Network Stream Paused for LevelChunk handling"`.

The paused packets are buffered rather than dropped.
`NetworkConnection::mPausedPackets` is a
`std::array<std::vector<PausedPacket>, 2>` at offset `+0x178`. On unpause, the
packets are replayed through `mResumedPackets`.


## The pause is recoverable because miss responses bypass buffering


A program-wide caller scan found exactly one caller of `getChannel()`. Its
return value is used solely as an index into
`NetworkConnection::mPausedChannels`, a `std::bitset<2>` at offset `+0x148`,
and `NetworkConnection::mPausedPackets`, a
`std::array<std::vector<PausedPacket>, 2>` at offset `+0x178`. This is
receive-side pause bookkeeping, not a RakNet ordering channel or any other
transport channel; it does not cause any packet to be transmitted
differently.

While bucket 0 is paused, received packets classified into it are buffered,
but packet ID 136 is classified into bucket 1 and processed immediately.
That exception allows a client stalled on a column to receive the blob
payloads required to complete that column and unpause.

## Vanilla has no timeout on the pause

A program-wide caller scan found exactly two callers of
`setConnectionChannelPaused`: the pause in
`queueHandleWorldChangePacket` and the unpause in `onChunkHandleCompleted`.
No watchdog or timeout call site exists for this pause.

Consequently, if a server never answers a blob miss for a column after a
world-change packet for that column has reached the queueing path, vanilla
leaves receive-side bucket 0 paused permanently. This is observed vanilla
behavior and is a remotely triggerable client hang.

## Chunk insertion is strictly sequence-ordered


Blob completion order does not determine insertion order. Packet arrival
sequence does: a stuck earlier column blocks insertion of later columns.

## Block-update discard case


## Cache status under pressure



A packet with an empty `mMissingIds` list and a populated `mFoundIds` list is
the normal vanilla shape for a chunk that is a full cache hit. A compliant
server must handle this packet.

Cinnabar now matches both status shapes: it emits no status packet when both
classified sets are empty, while a have-only classification still emits one
packet with an empty missing list.


## Concurrent transfers are limited by the server, not the client


| Network status | Maximum concurrent transfers |
| --- | ---: |
| `0` | 200 |
| `1` | 100 |
| `2` | 40 |
| any other value | 20 |


The 1.26.30 client has no corresponding enforcement cap. Its observed
self-limits are the shared 4,095-ID cache-status packet capacity and one status
packet per 5 ms. The similarly named
`ClientBlobCacheTrackingData::ActiveTransfersData::mMaximumAllowedActiveTransfers`
is performance-overlay telemetry mirroring the server tracker, not a client
enforcement point.

Mojang's public design note says that between one and eight transactions may be
concurrent. That figure **does not match the 1.26.30 binary** and is superseded
for this version by the server-side values above.

## Cache capacity, touch tracking, and eviction

**Observation.** Vanilla uses one persistent store: a LevelDB opened beneath
`AppPlatform::<vt+0x430>()` at `"blob_cache"`. There is no separate in-memory
blob store. Its keys are:

- `current_timestamp`, the monotonic timestamp base;
- `blob_<8-byte id>`, the payload; and
- `time_<8-byte id>`, the last-touch timestamp.





**Derived, not directly observed.** The timestamp unit is approximately 20 ms.
That duration is derived from reciprocal-multiply constants rather than a
directly observed duration literal.

## `mPendingChunks` is unbounded

**Observation.** A binary-wide review found exactly four operations on the
`mPendingChunks` map:

- `count` at `0x0795c633` in `queueHandleWorldChangePacket`;
- `_Try_emplace` at `0x07998dac` in `handle(LevelChunkPacket)`;
- `find` at `0x0795da81` in `onChunkHandleCompleted`; and
- `_Unchecked_erase` at `0x0795daa5` in `onChunkHandleCompleted`.

There is no size check. No `clear()` or `_Tidy` instantiation exists for this
map type in the binary, so even the general capability to clear the map was
not compiled in. The sole removal path is a chunk-task completion decrementing
the reference count to zero. Duplicate packets for a column increase the
count, and a server can grow the map without a bound. There is no abandon,
cancel, or timeout path.

`mConnectionPausedCallbacks` has the same unbounded container shape, but the
pause stops further ordinary packet processing, making that map self-limiting
to approximately one live entry in normal execution.

**Inference, not observation.** The `mPendingChunks` key contains a raw
`const Dimension*` and is never cleared. The verified lifetime facts imply
that a chunk task which never completes leaks its entry. If a later
`Dimension` reused the same address, its stale non-zero reference count could
pause a column bucket even though no corresponding task was in flight, with
nothing available to release it. Address reuse producing this collision was
not observed.

## Dimension change and disconnect behavior


Release remains driven by chunk-task completion.
`NetworkChunkInserter::onChunkHandleCompleted` compares the completed chunk's
dimension ID with the target dimension. A mismatch skips insertion, but still
calls `ClientNetworkHandler::onChunkHandleCompleted` unless
`isClientGeneratedChunk()` is true, and still increments
`mNextChunkSequenceID`.

Inside `ClientNetworkHandler::onChunkHandleCompleted`, the
`mPendingChunks` decrement happens first. A second gate compares the completed
chunk's dimension ID at `Dimension+0x1a0` with the local player's current
dimension ID. On mismatch, the deferred world-change callback is skipped, but
the map erase and receive-bucket unpause still occur. A dimension change can
therefore silently drop the queued world-change callback, but completion does
not leave the bucket wedged.

There is no timeout or disconnect hook that releases a pending pause. On
disconnect, destruction of the owning `NetworkConnection` destroys its
buffers.

## Corroborating public sources

These public sources corroborate parts of the binary observations but are not
the primary authority for this contract:

- Mojang's [blob-cache design note](https://gist.github.com/Tomcc/4be79d3eafcd158c5059abd4ab2e8d35)
  corroborates the maximum of 4,095 IDs in
  `ClientCacheBlobStatusPacket`. Its one-to-eight concurrent-transaction
  figure is superseded for 1.26.30 because it does not match this binary.
- A public [cache-poisoning disclosure](https://gist.github.com/JustTalDevelops/1abfdae7ab7618af2ec82f709ffa93bb)
  reports that the vanilla client no longer validates a blob payload against
  its hash.

## Cinnabar divergences and known gaps

Cinnabar rejects cached LevelChunk hash lists above 4,096 during wire decode, before resolver classification; this is a Cinnabar bound.

Cinnabar deliberately retains blob-payload hash validation even though
vanilla no longer performs it. This is a security-motivated divergence that
protects against cache poisoning.

Cinnabar does not currently replicate vanilla's permanent receive-side
bucket-0 pause as-is. Whether to reproduce vanilla's unbounded pause or
deliberately diverge with a bounded timeout remains open pending a decision
by the repository owner. This document does not resolve that choice.

Cinnabar currently resolves transactions out of order and uses per-column
ordering rather than vanilla's connection-wide receive pause. This is a known
divergence pending redesign, not a validated parity choice.

Cinnabar applies its own 256-retained-transaction memory-safety bound. This is
not a protocol or vanilla client limit. It sits above the observed server
maximum of 200; excess cached work is abandoned non-fatally and routed through
the existing chunk-resync recovery path instead of allowing remotely
controlled resolver growth.

Cinnabar also applies two bounds to the resolver's ordinary ready lane: 64
events and 32 MiB of accounted retained bytes. Neither is a vanilla limit. The
byte ceiling is twice Cinnabar's separate 16 MiB decoded-batch/deferred-raw-data
ceiling, leaving room for decoded container overhead. The event ceiling bounds
container and ordering metadata even when events have tiny or zero accounted
payloads. The session receive loop normally stops intake and drains retained
ordinary work before either direct-resolver ceiling is reached; a direct caller
at the ceiling receives explicit backpressure without dropping already
retained events.

Cinnabar limits one cached packet's reconstructed output payloads to 32 MiB.
Vanilla has no corresponding reconstruction limit. Before allocating output,
Cinnabar adds the uncached LevelChunk tail or successful SubChunk tails to the
known cached-blob length for every reference. Duplicate references are charged
once per occurrence because reconstruction copies every occurrence. A sum
above 32 MiB is abandoned non-fatally with truthful hit/miss classification and
the existing LevelChunk resync or scheduler-owned SubChunk recovery. The check
runs both for immediately ready cache hits and after the last miss response, so
unknown blob sizes cannot bypass it.

Cinnabar separately caps staged pinned blob payload at 32 MiB per unresolved
transaction. Vanilla has no corresponding limit. Cinnabar charges each unique
cached referenced blob on initial classification and each newly supplied
solicited miss before admitting that response to the cache. Crossing the
ceiling abandons the transaction non-fatally, releases every transaction pin,
and routes the affected LevelChunk or SubChunk through its existing recovery
path. Before this bound, a transaction with any missing reference bypassed the
final reconstruction projection while successive miss responses accumulated
pinned cache entries that trimming could not evict.

Cinnabar also caps aggregate accounted bytes across retained reconstructed
outputs at 32 MiB. Vanilla has no corresponding limit. The accounting includes
reconstructed payload vector capacities and their decoded packet containers.
If retaining the newly completed output would cross the ceiling, Cinnabar keeps
already retained outputs intact and abandons only the new transaction
non-fatally through the existing recovery path, so world data is not silently
dropped. Before this bound, the 256-retained-transaction ceiling constrained
only the number of ready outputs and allowed their aggregate payload memory to
grow far beyond one transaction's 32 MiB reconstruction ceiling.

Cinnabar additionally caps aggregate pending transaction bytes at 64 MiB via
`MAX_CLIENT_BLOB_PENDING_BYTES` (`crates/protocol/src/blob_cache.rs:34`),
enforced in `crates/protocol/src/blob_cache/resolver.rs:330`. Vanilla has no
corresponding limit. The accounting includes pending packet containers, inline
payloads, hash indexes, and cached ready containers. Crossing the ceiling
removes the new transaction non-fatally, releases all pins, increments
pressure/abandonment telemetry, and returns LevelChunk recovery. A
ready-transition overage uses the common abandonment path, which also releases
pins and queues recovery. SubChunk recovery remains scheduler-owned. This is a
Cinnabar memory-safety divergence.

Cinnabar's cache is byte-bounded only, admits blobs without an entry-count or
per-blob maximum, and uses a 100 MiB trigger, 80 MiB floor, and LRU last-touch
ordering. Its current in-memory implementation trims synchronously and uses
exact retained-byte accounting while preserving pinned/current entries.

Persistent LevelDB storage, vanilla's repeating 60-second trim task, its
timestamp write batching, and its 60-second recent-touch protection window are
not implemented in this tranche. These are known parity gaps. In particular,
Cinnabar's process-persistent shared in-memory cache is not equivalent to
vanilla's disk-persistent store or trim schedule.

Implementing vanilla's pause requires no channel or transport concept: only
a receive-side pause with the two-bucket classification described above.
Cinnabar's existing cache/ordinary lane split approximates that mechanism;
aligning the pause condition with vanilla's per-column reference count remains
open work.
