package proxy

import (
	"bytes"
	"sync"
	"sync/atomic"

	"github.com/sandertv/gophertunnel/minecraft/protocol"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

const (
	loadingOrderTraceLimit  = 16
	maxPublisherSavedChunks = 9216
)

// publisherOrderEvidence contains only the bounded numeric fields needed to
// compare publisher ordering at the upstream callback and relay boundaries.
type publisherOrderEvidence struct {
	Sequence          uint64
	CenterX           int32
	CenterY           int32
	CenterZ           int32
	RadiusBlocks      uint32
	SavedChunkCount   uint32
	LevelChunksBefore uint64
}

type publisherBoundarySnapshot struct {
	Updates       [loadingOrderTraceLimit]publisherOrderEvidence
	UpdateCount   uint64
	OverflowCount uint64
	InvalidCount  uint64
}

type loadingOrderTraceSnapshot struct {
	UpstreamCallback publisherBoundarySnapshot
	Relay            publisherBoundarySnapshot
}

type loadingOrderTrace struct {
	upstreamCallback publisherBoundaryTrace
	relay            publisherBoundaryTrace
}

type publisherBoundaryTrace struct {
	mu sync.Mutex

	sequence        atomic.Uint64
	levelChunkCount atomic.Uint64
	updates         [loadingOrderTraceLimit]publisherOrderEvidence
	updateCount     uint64
	overflowCount   uint64
	invalidCount    uint64
}

func (trace *loadingOrderTrace) observeUpstreamCallback(header packet.Header, payload []byte) {
	boundary := &trace.upstreamCallback
	sequence := atomicSaturatingIncrementValue(&boundary.sequence)
	switch header.PacketID {
	case packet.IDLevelChunk:
		atomicSaturatingIncrement(&boundary.levelChunkCount)
	case packet.IDNetworkChunkPublisherUpdate:
		update, ok := parsePublisherOrderEvidence(payload)
		if !ok {
			boundary.mu.Lock()
			saturatingIncrement(&boundary.invalidCount)
			boundary.mu.Unlock()
			return
		}
		boundary.record(update, sequence, boundary.levelChunkCount.Load())
	}
}

func (trace *loadingOrderTrace) observeRelay(value packet.Packet) {
	boundary := &trace.relay
	sequence := atomicSaturatingIncrementValue(&boundary.sequence)
	switch value := value.(type) {
	case *packet.LevelChunk:
		atomicSaturatingIncrement(&boundary.levelChunkCount)
	case *packet.NetworkChunkPublisherUpdate:
		boundary.record(publisherOrderEvidence{
			CenterX:         value.Position.X(),
			CenterY:         value.Position.Y(),
			CenterZ:         value.Position.Z(),
			RadiusBlocks:    value.Radius,
			SavedChunkCount: boundedUint32Len(len(value.SavedChunks)),
		}, sequence, boundary.levelChunkCount.Load())
	}
}

func (boundary *publisherBoundaryTrace) record(update publisherOrderEvidence, sequence, levelChunksBefore uint64) {
	boundary.mu.Lock()
	defer boundary.mu.Unlock()
	update.Sequence = sequence
	update.LevelChunksBefore = levelChunksBefore
	if boundary.updateCount < loadingOrderTraceLimit {
		boundary.updates[boundary.updateCount] = update
	} else {
		saturatingIncrement(&boundary.overflowCount)
	}
	saturatingIncrement(&boundary.updateCount)
}

func (trace *loadingOrderTrace) snapshot() loadingOrderTraceSnapshot {
	return loadingOrderTraceSnapshot{
		UpstreamCallback: trace.upstreamCallback.snapshot(),
		Relay:            trace.relay.snapshot(),
	}
}

func (boundary *publisherBoundaryTrace) snapshot() publisherBoundarySnapshot {
	boundary.mu.Lock()
	defer boundary.mu.Unlock()
	return publisherBoundarySnapshot{
		Updates:       boundary.updates,
		UpdateCount:   boundary.updateCount,
		OverflowCount: boundary.overflowCount,
		InvalidCount:  boundary.invalidCount,
	}
}

// parsePublisherOrderEvidence decodes the payload with the library codec and keeps only the
// bounded numeric fields; a list longer than maxPublisherSavedChunks counts as malformed.
func parsePublisherOrderEvidence(payload []byte) (publisherOrderEvidence, bool) {
	var update packet.NetworkChunkPublisherUpdate
	if !decodeObserved(&update, payload) || len(update.SavedChunks) > maxPublisherSavedChunks {
		return publisherOrderEvidence{}, false
	}
	return publisherOrderEvidence{
		CenterX:         update.Position.X(),
		CenterY:         update.Position.Y(),
		CenterZ:         update.Position.Z(),
		RadiusBlocks:    update.Radius,
		SavedChunkCount: boundedUint32Len(len(update.SavedChunks)),
	}, true
}

// decodeObserved decodes an upstream callback payload as the dialer's Conn does (no reader
// limits; lengths are still checked against the payload); malformed data reports false.
func decodeObserved(pk packet.Packet, payload []byte) (ok bool) {
	defer func() {
		if recover() != nil {
			ok = false
		}
	}()
	buffer := bytes.NewBuffer(payload)
	pk.Marshal(protocol.NewReader(buffer, 0, false))
	return buffer.Len() == 0
}

func boundedUint32Len(length int) uint32 {
	if uint64(length) > uint64(^uint32(0)) {
		return ^uint32(0)
	}
	return uint32(length)
}

func saturatingIncrement(value *uint64) {
	if *value != ^uint64(0) {
		*value++
	}
}

func atomicSaturatingIncrementValue(counter *atomic.Uint64) uint64 {
	for {
		current := counter.Load()
		if current == ^uint64(0) {
			return current
		}
		if counter.CompareAndSwap(current, current+1) {
			return current + 1
		}
	}
}
