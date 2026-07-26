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


## Corroborating public sources

These public sources corroborate parts of the binary observations but are not
the primary authority for this contract:

- Mojang's [blob-cache design note](https://gist.github.com/Tomcc/4be79d3eafcd158c5059abd4ab2e8d35)
  describes between one and eight concurrent transactions and a maximum of
  4,095 IDs in `ClientCacheBlobStatusPacket`.
- A public [cache-poisoning disclosure](https://gist.github.com/JustTalDevelops/1abfdae7ab7618af2ec82f709ffa93bb)
  reports that the vanilla client no longer validates a blob payload against
  its hash.

## Cinnabar divergences and open decisions

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

Implementing vanilla's pause requires no channel or transport concept: only
a receive-side pause with the two-bucket classification described above.
Cinnabar's existing cache/ordinary lane split approximates that mechanism;
aligning the pause condition with vanilla's per-column reference count remains
open work.
