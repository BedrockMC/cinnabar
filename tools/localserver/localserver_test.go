package main

import (
	"context"
	"io"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/df-mc/dragonfly/server/world"
)

func TestParseSettingsValidates(t *testing.T) {
	ok := []string{"-dir", "d", "-addr", "127.0.0.1:1", "-game-mode", "creative", "-difficulty", "hard"}
	s, err := parseSettings(ok, io.Discard)
	if err != nil || s.gameMode != "creative" || s.diff != "hard" {
		t.Fatalf("settings = %+v, %v", s, err)
	}
	for _, bad := range [][]string{
		{"-addr", "a"}, {"-dir", "d"},
		{"-dir", "d", "-addr", "a", "-game-mode", "hardcore"},
		{"-dir", "d", "-addr", "a", "-generator", "normal"}, // vanilla terrain is BDS-only
		{"-dir", "d", "-addr", "a", "-difficulty", "brutal"},
	} {
		if _, err := parseSettings(bad, io.Discard); err == nil {
			t.Fatalf("args %v must fail", bad)
		}
	}
}

func TestUserConfigIsOfflineAndScopedToDir(t *testing.T) {
	s := settings{dir: "/w", addr: "127.0.0.1:9", name: "n"}
	uc := s.userConfig()
	if uc.Server.AuthEnabled || uc.Network.Address != "127.0.0.1:9" || !uc.World.SaveData {
		t.Fatalf("config = %+v", uc)
	}
	for _, folder := range []string{uc.World.Folder, uc.Players.Folder, uc.Resources.Folder} {
		if !strings.HasPrefix(folder, "/w") {
			t.Fatalf("folder %q escapes world dir", folder)
		}
	}
}

func TestServeCommandsProtocol(t *testing.T) {
	var mu sync.Mutex
	var calls []bool
	done := make(chan struct{})
	go func() {
		serveCommands(context.Background(), strings.NewReader("pause\nbogus\nresume\nstop\npause\n"), func(p bool) {
			mu.Lock()
			calls = append(calls, p)
			mu.Unlock()
		})
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("serveCommands did not stop on stop")
	}
	mu.Lock()
	defer mu.Unlock()
	if len(calls) != 2 || !calls[0] || calls[1] {
		t.Fatalf("calls = %v", calls)
	}
}

func TestServeCommandsStopsOnEOF(t *testing.T) {
	done := make(chan struct{})
	go func() {
		serveCommands(context.Background(), strings.NewReader(""), func(bool) {})
		close(done)
	}()
	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("no stop on EOF")
	}
}

// Pause must suspend every dimension and resume must keep each world's own settings.
func TestSetPausedSuspendsAndRestoresWorlds(t *testing.T) {
	cycling, stopped := world.Config{}.New(), world.Config{}.New()
	defer cycling.Close()
	defer stopped.Close()
	stopped.StopTime()
	worlds := []*world.World{cycling, stopped}

	setPaused(worlds, true)
	for _, w := range worlds {
		if !w.Paused() {
			t.Fatal("world not paused")
		}
	}
	setPaused(worlds, false)
	if cycling.Paused() || stopped.Paused() {
		t.Fatal("world still paused after resume")
	}
	if !cycling.TimeCycle() || stopped.TimeCycle() {
		t.Fatalf("resume changed time cycle: cycling=%v stopped=%v", cycling.TimeCycle(), stopped.TimeCycle())
	}
}
