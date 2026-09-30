package proxy

import (
	"context"
	"io"
	"net"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

// Run: go test ./proxy -run '^$' -bench RelayCompare -benchmem
// Compares the batched pump with a per-packet baseline on flushes, allocations,
// first-packet latency and shutdown latency against a stalled peer.

type replaySource struct {
	batches [][]packet.Packet
	next    int
}

func (s *replaySource) ReadBatch() ([]packet.Packet, error) {
	if s.next >= len(s.batches) {
		return nil, io.EOF
	}
	s.next++
	return s.batches[s.next-1], nil
}
func (*replaySource) WritePacketImmediate(...packet.Packet) error { return nil }
func (*replaySource) Flush() error                                { return nil }
func (*replaySource) Abort() error                                { return nil }
func (*replaySource) Close() error                                { return nil }

// discardSink counts wire flushes and packets; stall makes writes block until close.
type discardSink struct {
	flushes, packets atomic.Int64
	firstWrite       chan struct{}
	firstOnce        sync.Once
	stall            bool
	closed           chan struct{}
	closeOnce        sync.Once
}

func newDiscardSink(stall bool) *discardSink {
	return &discardSink{firstWrite: make(chan struct{}), stall: stall, closed: make(chan struct{})}
}

func (s *discardSink) ReadBatch() ([]packet.Packet, error) {
	<-s.closed
	return nil, net.ErrClosed
}

func (s *discardSink) WritePacketImmediate(packets ...packet.Packet) error {
	s.firstOnce.Do(func() { close(s.firstWrite) })
	if s.stall {
		<-s.closed
		return net.ErrClosed
	}
	s.flushes.Add(1)
	s.packets.Add(int64(len(packets)))
	return nil
}
func (*discardSink) Flush() error { return nil }
func (s *discardSink) Abort() error {
	s.closeOnce.Do(func() { close(s.closed) })
	return nil
}
func (s *discardSink) Close() error { return s.Abort() }

func compareBatches(batches, perBatch int) [][]packet.Packet {
	out := make([][]packet.Packet, batches)
	for i := range out {
		out[i] = make([]packet.Packet, perBatch)
		for j := range out[i] {
			out[i][j] = &packet.NetworkStackLatency{Timestamp: int64(j)}
		}
	}
	return out
}

// perPacketPump is the pre-batch baseline: one write and flush per packet.
func perPacketPump(source *replaySource, sink *discardSink) {
	for {
		batch, err := source.ReadBatch()
		if err != nil {
			return
		}
		for _, value := range batch {
			_ = sink.WritePacketImmediate(value)
		}
	}
}

func BenchmarkRelayCompareThroughput(b *testing.B) {
	batches := compareBatches(200, 50)
	for name, run := range map[string]func(*replaySource, *discardSink){
		"batched":   func(src *replaySource, sink *discardSink) { _ = pumpPackets(src, sink, false) },
		"perPacket": perPacketPump,
	} {
		b.Run(name, func(b *testing.B) {
			b.ReportAllocs()
			var flushes int64
			for i := 0; i < b.N; i++ {
				sink := newDiscardSink(false)
				run(&replaySource{batches: batches}, sink)
				flushes = sink.flushes.Load()
			}
			b.ReportMetric(float64(flushes), "flushes/op")
		})
	}
}

func BenchmarkRelayCompareFirstPacketLatency(b *testing.B) {
	batches := compareBatches(1, 50)
	var total time.Duration
	for i := 0; i < b.N; i++ {
		sink := newDiscardSink(false)
		start := time.Now()
		go func() { _ = pumpPackets(&replaySource{batches: batches}, sink, false) }()
		<-sink.firstWrite
		total += time.Since(start)
	}
	b.ReportMetric(float64(total.Nanoseconds())/float64(b.N), "first-write-ns")
}

func BenchmarkRelayCompareShutdownWithStalledPeer(b *testing.B) {
	batches := compareBatches(1, 50)
	var total time.Duration
	for i := 0; i < b.N; i++ {
		down, sink := &replaySource{batches: batches}, newDiscardSink(true)
		ctx, cancel := context.WithCancel(context.Background())
		done := make(chan struct{})
		go func() { _ = relayPackets(ctx, sink, down); close(done) }()
		<-sink.firstWrite
		start := time.Now()
		cancel()
		<-done
		total += time.Since(start)
	}
	b.ReportMetric(float64(total.Nanoseconds())/float64(b.N), "shutdown-ns")
}
