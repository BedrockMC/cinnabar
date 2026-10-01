package proxy

import (
	"errors"
	"io"
	"slices"
	"sync/atomic"
	"testing"
	"time"

	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

type countedFlushSink struct {
	*fakeUpstream
	flushes  atomic.Int32
	flushed  chan struct{}
	failure  error
	writeErr error
}

// WritePacket can fail independently of the destination's flush.
func (s *countedFlushSink) WritePacket(value packet.Packet) error {
	if s.writeErr != nil {
		return s.writeErr
	}
	return s.fakeUpstream.WritePacket(value)
}

// Flush counts even empty attempts and signals after transport submission.
func (s *countedFlushSink) Flush() error {
	s.flushes.Add(1)
	err := s.fakeUpstream.Flush()
	if s.flushed != nil {
		s.flushed <- struct{}{}
	}
	return errors.Join(err, s.failure)
}

// controlledReader uses manual ticks so assertions depend on events, not scheduler timing.
func controlledReader(t *testing.T, source *fakeDownstream, sink packetSession, upstream bool) (*packetReader, chan time.Time) {
	t.Helper()
	source.useBatchReads = true
	reader := newPacketReader(source, sink, upstream, time.Hour)
	reader.idle.Stop()
	ticks := make(chan time.Time, 1)
	reader.idle.C = ticks
	t.Cleanup(func() { reader.Close(); _ = source.Close() })
	return reader, ticks
}

func TestRelayCoalescesRequestsAtStalledBatchBoundary(t *testing.T) {
	src := newFakeDownstream(nil)
	sink := &countedFlushSink{fakeUpstream: newFakeUpstream(nil)}
	reader, _ := controlledReader(t, src, sink, false)
	want := stamps(1)[0]
	src.batchReads <- batchResult{packets: want}
	batch, err := reader.Read()
	if err != nil {
		t.Fatal(err)
	}
	entered, release, done := make(chan struct{}), make(chan struct{}), make(chan error, 1)
	go func() {
		_ = sink.WritePacket(batch[0])
		close(entered)
		<-release
		_ = sink.WritePacket(batch[1])
		done <- reader.Flush()
	}()
	<-entered
	for range 100 {
		reader.RequestFlush()
	}
	if got := len(reader.flushRequests); got != 1 {
		t.Fatalf("queued requests = %d", got)
	}
	if sink.flushes.Load() != 0 {
		t.Fatal("request split the stalled batch")
	}
	close(release)
	if err := <-done; err != nil {
		t.Fatal(err)
	}
	if sink.flushes.Load() != 1 || len(reader.flushRequests) != 0 {
		t.Fatal("boundary did not satisfy exactly one coalesced request")
	}
	if got := sink.flushedBatches(); len(got) != 1 || !slices.Equal(got[0], want) {
		t.Fatalf("delivered = %v", got)
	}
	// A subsequent read must not service a stale request, even if EOF wins its select.
	src.batchReads <- batchResult{err: io.EOF}
	if _, err := reader.Read(); !errors.Is(err, io.EOF) {
		t.Fatal(err)
	}
	if sink.flushes.Load() != 1 {
		t.Fatal("stale request caused another flush")
	}
}

func TestRelayIdleWriteLeavesOnFirstTick(t *testing.T) {
	src := newFakeDownstream(nil)
	sink := &countedFlushSink{fakeUpstream: newFakeUpstream(nil), flushed: make(chan struct{}, 1)}
	reader, ticks := controlledReader(t, src, sink, false)
	out := &packet.NetworkStackLatency{Timestamp: 9}
	if err := sink.WritePacket(out); err != nil {
		t.Fatal(err)
	}
	done := make(chan error, 1)
	go func() { _, err := reader.Read(); done <- err }()
	ticks <- time.Now()
	select {
	case <-sink.flushed:
	case <-time.After(time.Second):
		t.Fatal("first idle tick did not flush")
	}
	if got := sink.flushedBatches(); len(got) != 1 || got[0][0] != out || sink.flushes.Load() != 1 {
		t.Fatal("first tick did not deliver the buffered write exactly once")
	}
	src.batchReads <- batchResult{err: io.EOF}
	if err := <-done; !errors.Is(err, io.EOF) {
		t.Fatal(err)
	}
}

func TestRelayFailureAttribution(t *testing.T) {
	for _, upstream := range []bool{false, true} {
		for _, operation := range []string{"read", "write", "idle", "request", "boundary"} {
			t.Run(operation+map[bool]string{true: "-upstream", false: "-downstream"}[upstream], func(t *testing.T) {
				failure := &minecraft.DisconnectPacketError{Message: "injected " + operation}
				src := newFakeDownstream(nil)
				sink := &countedFlushSink{fakeUpstream: newFakeUpstream(nil), failure: failure}
				reader, ticks := controlledReader(t, src, sink, upstream)
				var err error
				switch operation {
				case "read":
					src.batchReads <- batchResult{err: failure}
					_, err = reader.Read()
				case "idle":
					ticks <- time.Now()
					_, err = reader.Read()
				case "request":
					reader.RequestFlush()
					_, err = reader.Read()
				case "boundary":
					err = reader.Flush()
				case "write":
					src = newFakeDownstream(nil)
					src.useBatchReads = true
					defer src.Close()
					sink.failure = nil
					sink.writeErr = failure
					src.batchReads <- batchResult{packets: stamps(1)[0]}
					err = pumpPackets(src, sink, !upstream)
				}
				var attributed *upstreamRelayDisconnect
				wantUpstream := !upstream
				if operation == "read" {
					wantUpstream = upstream
				}
				if !errors.Is(err, failure) || errors.As(err, &attributed) != wantUpstream {
					t.Fatalf("error = %v; upstream attribution wanted %t", err, wantUpstream)
				}
			})
		}
	}
}
