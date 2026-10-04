package proxy

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/df-mc/go-nethernet/endpoint"
	"github.com/hashimthearab/rust-mcbe/core/localworld"
	"github.com/sandertv/gophertunnel/minecraft/protocol"
)

func TestLocalBDSUsesHTTPStatusWithoutOnlineResolution(t *testing.T) {
	t.Parallel()
	want := endpoint.Status{ServerName: "Local fixture", Protocol: protocol.CurrentProtocol, Version: protocol.CurrentVersion, MaxPlayerCount: 8}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet || r.URL.Path != "/v1/join" {
			http.Error(w, "unexpected endpoint", http.StatusNotFound)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(want)
	}))
	t.Cleanup(server.Close)
	resolve := withLocalTarget(func(context.Context) (localworld.ConnectionTarget, bool, error) {
		return localworld.ConnectionTarget{Address: server.Listener.Addr().String(), Transport: localworld.TransportNetherNetHTTP}, true, nil
	}, func(context.Context) (*resolvedUpstreamTarget, error) {
		t.Fatal("local BDS must not resolve Xbox or online catalog targets")
		return nil, errors.New("unexpected online resolution")
	})
	target, err := resolve(t.Context())
	if err != nil || target.address != server.URL {
		t.Fatalf("target = %+v, %v", target, err)
	}
	pong, err := target.network.PingContext(t.Context(), target.address)
	if err != nil {
		t.Fatal(err)
	}
	got, err := endpoint.RakNetPongData(pong)
	if err != nil || got != want {
		t.Fatalf("HTTP status = %+v, %v, want %+v", got, err, want)
	}
	ctx, cancel := context.WithCancel(t.Context())
	cancel()
	if _, err := target.network.PingContext(ctx, target.address); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled status request = %v", err)
	}
}

func TestLocalNetherNetRejectsUnspecifiedTransportAndNonLoopbackURLs(t *testing.T) {
	t.Parallel()
	for _, test := range []struct {
		name      string
		address   string
		transport localworld.Transport
	}{
		{name: "missing transport", address: "127.0.0.1:5000"},
		{name: "remote host", address: "192.0.2.10:5000", transport: localworld.TransportNetherNetHTTP},
		{name: "zero port", address: "127.0.0.1:0", transport: localworld.TransportNetherNetHTTP},
		{name: "overflow port", address: "127.0.0.1:65536", transport: localworld.TransportNetherNetHTTP},
		{name: "url injection", address: "127.0.0.1:5000/path", transport: localworld.TransportNetherNetHTTP},
	} {
		t.Run(test.name, func(t *testing.T) {
			t.Parallel()
			resolve := withLocalTarget(func(context.Context) (localworld.ConnectionTarget, bool, error) {
				return localworld.ConnectionTarget{Address: test.address, Transport: test.transport}, true, nil
			}, onlineStub("online"))
			if target, err := resolve(t.Context()); err == nil || target != nil {
				t.Fatalf("invalid local target accepted: %+v, %v", target, err)
			}
		})
	}
	resolve := withLocalTarget(func(context.Context) (localworld.ConnectionTarget, bool, error) {
		return localworld.ConnectionTarget{Address: "[::1]:5000", Transport: localworld.TransportNetherNetHTTP}, true, nil
	}, onlineStub("online"))
	if target, err := resolve(t.Context()); err != nil || !strings.HasPrefix(target.address, "http://[::1]:") {
		t.Fatalf("IPv6 loopback target = %+v, %v", target, err)
	}
}

func TestLocalBDSDoesNotFollowStatusRedirects(t *testing.T) {
	t.Parallel()
	destination := httptest.NewServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {
		t.Error("local BDS status must not cross an HTTP redirect")
	}))
	t.Cleanup(destination.Close)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		http.Redirect(w, r, destination.URL, http.StatusTemporaryRedirect)
	}))
	t.Cleanup(server.Close)
	resolve := withLocalTarget(func(context.Context) (localworld.ConnectionTarget, bool, error) {
		return localworld.ConnectionTarget{Address: server.Listener.Addr().String(), Transport: localworld.TransportNetherNetHTTP}, true, nil
	}, onlineStub("online"))
	target, err := resolve(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	if _, err := target.network.PingContext(t.Context(), target.address); err == nil {
		t.Fatal("local BDS status redirect was accepted")
	}
}
