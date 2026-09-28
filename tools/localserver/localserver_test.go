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
	ok := []string{"-dir", "d", "-addr", "127.0.0.1:1", "-game-mode", "creative", "-generator", "flat", "-difficulty", "hard", "-seed", "-9"}
	s, err := parseSettings(ok, io.Discard)
	if err != nil || s.seed != -9 || s.generator != "flat" {
		t.Fatalf("settings = %+v, %v", s, err)
	}
	for _, bad := range [][]string{
		{"-addr", "a"}, {"-dir", "d"},
		{"-dir", "d", "-addr", "a", "-game-mode", "hardcore"},
		{"-dir", "d", "-addr", "a", "-generator", "void"},
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

func TestNormalHeightIsDeterministicPerSeedAndBounded(t *testing.T) {
	a, b, other := normal{seed: 42}, normal{seed: 42}, normal{seed: 43}
	differs := false
	for x := -200; x < 200; x += 13 {
		for z := -200; z < 200; z += 17 {
			h := a.height(x, z)
			if h != b.height(x, z) {
				t.Fatalf("height(%d,%d) not deterministic", x, z)
			}
			if h < baseHeight-heightSpread-1 || h > baseHeight+heightSpread+1 {
				t.Fatalf("height %d out of bounds", h)
			}
			differs = differs || h != other.height(x, z)
		}
	}
	if !differs {
		t.Fatal("different seeds produced identical terrain")
	}
}

func TestNormalHeightIsContinuousAcrossChunkBorders(t *testing.T) {
	n := normal{seed: 7}
	for x := -64; x < 64; x++ {
		if d := n.height(x, 5) - n.height(x+1, 5); d > 4 || d < -4 {
			t.Fatalf("cliff of %d between x=%d and x=%d", d, x, x+1)
		}
	}
}

func TestBlockAtLayersColumn(t *testing.T) {
	n := normal{bedrock: 1, stone: 2, dirt: 3, grass: 4, sand: 5, water: 6, air: 0}
	const min, h = -64, 70
	want := map[int]uint32{min: 1, 0: 2, h - dirtDepth - 1: 2, h - 1: 3, h: 4, h + 1: 0}
	for y, rid := range want {
		if got := n.blockAt(y, min, h); got != rid {
			t.Fatalf("y=%d: got %d want %d", y, got, rid)
		}
	}
	if got := n.blockAt(seaLevel, min, seaLevel-5); got != n.water {
		t.Fatalf("submerged column top = %d", got)
	}
	if got := n.blockAt(seaLevel-5, min, seaLevel-5); got != n.sand {
		t.Fatalf("seabed = %d", got)
	}
}

func TestDefaultSpawnIsDryLand(t *testing.T) {
	n := normal{seed: 99}
	if pos := n.DefaultSpawn(world.Overworld); pos[1] <= seaLevel {
		t.Fatalf("spawn %v is not above sea level", pos)
	}
}

var _ world.Generator = normal{}

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
