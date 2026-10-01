package proxy

import (
	"context"
	"fmt"
	"io"
	"log/slog"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
	"time"

	"github.com/hashimthearab/rust-mcbe/core/internal/streamnet"
	"github.com/sandertv/gophertunnel/minecraft"
	"github.com/sandertv/gophertunnel/minecraft/protocol/packet"
)

// TestProxyRustStartupHarness runs the production relay against an offline scripted upstream.
func TestProxyRustStartupHarness(t *testing.T) {
	scenario := os.Getenv("CINNABAR_STARTUP_FIXTURE")
	if scenario == "" {
		t.Skip("started by the Rust protocol integration test")
	}
	socketDir, err := externalRustClientSocketDir()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	upstreamNetwork := streamnet.New(filepath.Join(t.TempDir(), "upstream"))
	upstream, err := (minecraft.ListenConfig{AuthenticationDisabled: true, FlushRate: -1, ErrorLog: slog.New(slog.DiscardHandler)}).ListenNetwork(upstreamNetwork, "")
	if err != nil {
		t.Fatal(err)
	}
	defer upstream.Close()
	serverDone := make(chan error, 1)
	upstreamConnection := make(chan *minecraft.Conn, 1)
	go func() {
		accepted, err := upstream.Accept()
		if err != nil {
			serverDone <- err
			return
		}
		conn := accepted.(*minecraft.Conn)
		err = runRustStartupScript(conn, scenario)
		_ = conn.Close()
		serverDone <- err
	}()
	connections := newPreparedConnections("unused.invalid:19132", nil, slog.New(slog.DiscardHandler))
	connections.resolveTarget = func(context.Context) (*resolvedUpstreamTarget, error) {
		return &resolvedUpstreamTarget{network: upstreamNetwork}, nil
	}
	connections.dialTarget = func(ctx context.Context, target *resolvedUpstreamTarget, dialer minecraft.Dialer) (upstreamSession, error) {
		conn, err := dialer.DialContextNetwork(ctx, target.network, "")
		if err == nil {
			upstreamConnection <- conn
		}
		return conn, err
	}
	listener, err := localListenConfig(connections.prepare).ListenNetwork(streamnet.New(socketDir), "")
	if err != nil {
		t.Fatal(err)
	}
	defer listener.Close()
	defer connections.finishShutdown()
	go func() {
		accepted, err := listener.Accept()
		if err != nil {
			return
		}
		downstream := accepted.(*minecraft.Conn)
		prepared, err := takePreparedAfterAccept(connections, downstream)
		if err != nil || prepared == nil {
			_ = downstream.Close()
			return
		}
		if strings.HasPrefix(scenario, "transfer") {
			// All upstream frames and EOF must be queued before either relay pump starts.
			conn := <-upstreamConnection
			select {
			case <-conn.Context().Done():
			case <-ctx.Done():
			}
		}
		_ = servePreparedConnection(ctx, downstream, prepared)
	}()
	fmt.Printf("RUST_MCBE_EXTERNAL_READY=%s\n", socketDir)
	if _, err := io.Copy(io.Discard, os.Stdin); err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-serverDone:
		if err != nil {
			t.Fatal(err)
		}
	case <-ctx.Done():
		t.Fatal(ctx.Err())
	}
	connections.beginShutdown()
}

// runRustStartupScript asserts each client message at the upstream end of both relay legs.
func runRustStartupScript(conn *minecraft.Conn, scenario string) error {
	startup := relayFixtureStartup()
	transfer := &packet.Transfer{Address: "next.example.test", Port: 19133}
	if scenario == "transfer-batch" {
		return conn.WritePacketImmediate(startup[0], transfer)
	}
	if err := conn.WritePacketImmediate(startup[0]); err != nil {
		return err
	}
	if scenario == "transfer" {
		return conn.WritePacketImmediate(transfer)
	}
	for _, expected := range []packet.Packet{&packet.RequestChunkRadius{ChunkRadius: 16, MaxChunkRadius: 16}, &packet.ServerBoundLoadingScreen{Type: packet.LoadingScreenTypeStart}} {
		if err := expectStartupPacket(conn, expected); err != nil {
			return err
		}
	}
	// Readiness prerequisites arrive only after both initial client messages.
	if err := conn.WritePacketImmediate(startup[1:]...); err != nil {
		return err
	}
	if err := expectStartupPacket(conn, &packet.NetworkStackLatency{Timestamp: 100}); err != nil {
		return fmt.Errorf("initialized before presentation readiness: %w", err)
	}
	if err := conn.WritePacketImmediate(&packet.SetTime{Time: 200}); err != nil {
		return err
	}
	for _, expected := range []packet.Packet{&packet.ServerBoundLoadingScreen{Type: packet.LoadingScreenTypeEnd}, &packet.SetLocalPlayerAsInitialised{EntityRuntimeID: 42}, &packet.NetworkStackLatency{Timestamp: 300}} {
		if err := expectStartupPacket(conn, expected); err != nil {
			return err
		}
	}
	return conn.WritePacketImmediate(&packet.SetTime{Time: 400})
}

// expectStartupPacket compares complete decoded packets, including order and runtime identity.
func expectStartupPacket(conn *minecraft.Conn, expected packet.Packet) error {
	actual, err := conn.ReadPacket()
	if err != nil {
		return err
	}
	if !reflect.DeepEqual(actual, expected) {
		return fmt.Errorf("received %#v, want %#v", actual, expected)
	}
	return nil
}
