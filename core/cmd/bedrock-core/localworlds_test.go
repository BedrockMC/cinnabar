package main

import (
	"io"
	"strings"
	"testing"
)

func TestLocalWorldsFlagsRequireControlStatus(t *testing.T) {
	if _, err := parseFlags([]string{"-local-worlds-dir", "w"}, io.Discard); err == nil || !strings.Contains(err.Error(), "-control-status") {
		t.Fatalf("err = %v", err)
	}
	if _, err := parseFlags([]string{"-local-server-bin", "b"}, io.Discard); err == nil {
		t.Fatal("local-server-bin without worlds dir must fail")
	}
	opts, err := parseFlags([]string{"-control-status", "-local-worlds-dir", "w", "-local-server-bin", "b"}, io.Discard)
	if err != nil || opts.localWorldsDir != "w" || opts.localServerBin != "b" {
		t.Fatalf("opts = %+v, %v", opts, err)
	}
}

func TestOpenLocalWorldsFailsClosedWhenBinaryMissing(t *testing.T) {
	_, err := openLocalWorlds(options{localWorldsDir: t.TempDir(), localServerBin: "/nonexistent/bedrock-local-server", localBackend: "dragonfly"}, newLifecycleLogger(io.Discard))
	if err == nil || !strings.Contains(err.Error(), "make local-server") {
		t.Fatalf("err = %v", err)
	}
}

func TestLocalBackendFlagIsValidated(t *testing.T) {
	if _, err := parseFlags([]string{"-local-backend", "java"}, io.Discard); err == nil {
		t.Fatal("unknown backend must fail")
	}
	opts, err := parseFlags(nil, io.Discard)
	if err != nil || opts.localBackend != "auto" {
		t.Fatalf("opts = %+v, %v", opts, err)
	}
}

func TestBDSDefaultDoesNotRequireDragonflyBinary(t *testing.T) {
	manager, err := openLocalWorlds(options{localWorldsDir: t.TempDir(), localServerBin: "/nonexistent/x", localBackend: "bds"}, newLifecycleLogger(io.Discard))
	if err != nil || manager == nil {
		t.Fatalf("manager = %v, err = %v", manager, err)
	}
}
