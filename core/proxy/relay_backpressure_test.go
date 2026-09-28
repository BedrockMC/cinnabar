package proxy

import (
	"context"
	"errors"
	"io"
	"net"
	"reflect"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

// countingSource counts ReadBatch calls on the wrapped session.
type countingSource struct {
	*fakeDownstream
	reads atomic.Int32
}

func (s *countingSource) ReadBatch() ([]packet.Packet, error) {
	s.reads.Add(1)
	return s.fakeDownstream.ReadBatch()
}

// gatedSink blocks writes until gate closes or the session is torn down.
type gatedSink struct {
	*fakeUpstream
	gate    chan struct{}
	entered chan struct{}
}

func newGatedSink() *gatedSink {
	return &gatedSink{fakeUpstream: newFakeUpstream(nil), gate: make(chan struct{}), entered: make(chan struct{}, 64)}
}

func (s *gatedSink) WritePacketImmediate(packets ...packet.Packet) error {
	s.entered <- struct{}{}
	select {
	case <-s.gate:
	case <-s.closed:
		return net.ErrClosed
	}
	return s.fakeUpstream.WritePacketImmediate(packets...)
}

func stamps(n int) [][]packet.Packet {
	batches := make([][]packet.Packet, n)
	for i := range batches {
		batches[i] = []packet.Packet{&packet.NetworkStackLatency{Timestamp: int64(i)}, &packet.NetworkStackLatency{Timestamp: int64(i) + 100}}
	}
	return batches
}

func TestRelaySlowReaderBoundsReadAheadAndStaysLossless(t *testing.T) {
	src := &countingSource{fakeDownstream: newFakeDownstream(nil)}
	src.useBatchReads = true
	sink := newGatedSink()
	want := stamps(5)
	for _, b := range want {
		src.batchReads <- batchResult{packets: b}
	}
	src.batchReads <- batchResult{err: io.EOF}

	done := make(chan error, 1)
	go func() { done <- pumpPackets(src, sink, true) }()
	<-sink.entered
	time.Sleep(50 * time.Millisecond)
	if got := src.reads.Load(); got != 1 {
		t.Fatalf("source batches read while sink stalled = %d, want 1", got)
	}
	close(sink.gate)
	if err := <-done; !errors.Is(err, io.EOF) {
		t.Fatalf("pumpPackets() error = %v, want EOF", err)
	}
	got := sink.flushedBatches()
	if len(got) != len(want) {
		t.Fatalf("delivered %d batches, want %d", len(got), len(want))
	}
	for i := range want {
		if !slices.Equal(got[i], want[i]) {
			t.Fatalf("batch %d reordered or altered: %#v", i, got[i])
		}
	}
}

func TestRelayCancellationReleasesStalledWriter(t *testing.T) {
	down := newFakeDownstream(nil)
	down.useBatchReads = true
	sink := newGatedSink()
	down.batchReads <- batchResult{packets: stamps(1)[0]}
	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() { done <- relayPackets(ctx, down, sink) }()
	<-sink.entered
	cancel()
	select {
	case err := <-done:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("relayPackets() error = %v, want cancellation", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("relay did not shut down with a stalled writer")
	}
}

func TestRelayForwardsPartialBatchBeforeMidBatchDecodeClose(t *testing.T) {
	down := newFakeDownstream(nil)
	up := newFakeUpstream(nil)
	up.useBatchReads = true
	delivered := stamps(1)[0]
	closeErr := errors.New("decode closed connection")
	up.batchReads <- batchResult{packets: delivered}
	up.batchReads <- batchResult{err: closeErr}

	err := relayPackets(context.Background(), down, up)
	if !errors.Is(err, closeErr) {
		t.Fatalf("relay error = %v, want decode close error", err)
	}
	batches := down.flushedBatches()
	if len(batches) != 1 || !slices.Equal(batches[0], delivered) {
		t.Fatalf("packets before the offending one were not delivered as one batch: %#v", batches)
	}
	if !down.isClosed() || !up.isClosed() {
		t.Fatal("relay left a session open after decode close")
	}
}

func TestRelaySkipsEmptyBatchesWithoutFlushing(t *testing.T) {
	down := newFakeDownstream(nil)
	up := newFakeUpstream(nil)
	down.useBatchReads = true
	want := stamps(1)[0]
	down.batchReads <- batchResult{packets: nil}
	down.batchReads <- batchResult{packets: want}
	down.batchReads <- batchResult{err: io.EOF}
	if err := pumpPackets(down, up, true); !errors.Is(err, io.EOF) {
		t.Fatalf("pumpPackets() error = %v, want EOF", err)
	}
	if got := up.flushedBatches(); len(got) != 1 || !slices.Equal(got[0], want) {
		t.Fatalf("batches = %#v, want one batch %#v", got, want)
	}
}

func TestRelayFlushesDeferredLoadingStartOnSourceClose(t *testing.T) {
	for name, terminal := range map[string]error{"eof": io.EOF, "transport": errors.New("reset")} {
		t.Run(name, func(t *testing.T) {
			down := newFakeDownstream(nil)
			up := newFakeUpstream(nil)
			down.useBatchReads = true
			start := &packet.ServerBoundLoadingScreen{Type: packet.LoadingScreenTypeStart}
			down.batchReads <- batchResult{packets: []packet.Packet{start}}
			down.batchReads <- batchResult{err: terminal}
			if err := pumpPackets(down, up, true); !errors.Is(err, terminal) {
				t.Fatalf("pumpPackets() error = %v, want %v", err, terminal)
			}
			batches := up.flushedBatches()
			if len(batches) != 1 || len(batches[0]) != 1 || batches[0][0] != start {
				t.Fatalf("deferred Start was lost or merged: %#v", batches)
			}
		})
	}
}

func TestRelayKeepsBatchBoundaryBeforeUpstreamDisconnect(t *testing.T) {
	down := newFakeDownstream(nil)
	up := newFakeUpstream(nil)
	up.useBatchReads = true
	before := stamps(1)[0]
	reason := &minecraft.DisconnectPacketError{Message: "server message", FilteredMessage: "filtered"}
	up.batchReads <- batchResult{packets: before}
	up.batchReads <- batchResult{err: reason}
	if err := relayPackets(context.Background(), down, up); !errors.Is(err, reason) {
		t.Fatalf("relay error = %v, want disconnect", err)
	}
	batches := down.flushedBatches()
	if len(batches) != 2 || !slices.Equal(batches[0], before) || len(batches[1]) != 1 || !reflect.DeepEqual(batches[1][0], reason.Packet()) {
		t.Fatalf("batches = %#v, want [pre-disconnect batch][disconnect]", batches)
	}
}
