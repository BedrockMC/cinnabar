package proxy

import (
	"errors"
	"io"
	"slices"
	"testing"

	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

func TestTransferStateRecordsNextUpstreamAndNotifies(t *testing.T) {
	var state TransferState
	var seen []TransferTarget
	state.OnTransfer = func(target TransferTarget) { seen = append(seen, target) }
	if got := state.Upstream("first:19132"); got != "first:19132" {
		t.Fatalf("Upstream() = %q before any transfer", got)
	}
	if err := state.Record(TransferTarget{Host: " [::1] ", Port: 19133}); err != nil {
		t.Fatal(err)
	}
	if got := state.Upstream("first:19132"); got != "[::1]:19133" {
		t.Fatalf("Upstream() = %q, want [::1]:19133", got)
	}
	if want := []TransferTarget{{Host: "::1", Port: 19133}}; !slices.Equal(seen, want) {
		t.Fatalf("notified %v, want %v", seen, want)
	}
}

func TestTransferStateIgnoresUnusableTargets(t *testing.T) {
	var state TransferState
	calls := 0
	state.OnTransfer = func(TransferTarget) { calls++ }
	for _, target := range []TransferTarget{{Port: 19132}, {Host: "h"}} {
		if err := state.Record(target); err == nil {
			t.Fatalf("Record(%v) accepted an unusable target", target)
		}
	}
	if calls != 0 || state.Upstream("first:1") != "first:1" {
		t.Fatal("unusable transfer changed state")
	}
}

func TestObserveTransfersRecordsAndStillRelays(t *testing.T) {
	up := newFakeUpstream(nil)
	transfer := &packet.Transfer{Address: "next.example", Port: 19140}
	bad := &packet.Transfer{Port: 19140}
	up.reads <- packetResult{packet: bad}
	up.reads <- packetResult{packet: transfer}
	up.reads <- packetResult{err: io.EOF}
	var state TransferState
	session := observeTransfers(up, &state, nil)
	for _, want := range []packet.Packet{bad, transfer} {
		batch, err := session.ReadBatch()
		if err != nil || len(batch) != 1 || batch[0] != want {
			t.Fatalf("ReadBatch() = %v, %v; want the packet unchanged", batch, err)
		}
	}
	if got := state.Upstream("first:1"); got != "next.example:19140" {
		t.Fatalf("Upstream() = %q, want the valid transfer", got)
	}
	if _, err := session.ReadBatch(); !errors.Is(err, io.EOF) {
		t.Fatalf("ReadBatch() error = %v, want EOF", err)
	}
}

func TestObserveTransfersWithoutStateIsPassThrough(t *testing.T) {
	up := newFakeUpstream(nil)
	if observeTransfers(up, nil, nil) != upstreamSession(up) {
		t.Fatal("nil state must not wrap the session")
	}
}
