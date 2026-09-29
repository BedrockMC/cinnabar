package localworld

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func newTestStore(t *testing.T) *Store {
	t.Helper()
	store, err := OpenStore(filepath.Join(t.TempDir(), "worlds"))
	if err != nil {
		t.Fatal(err)
	}
	return store
}

func TestCreateAppliesDefaultsAndPersists(t *testing.T) {
	store := newTestStore(t)
	world, err := store.Create(Spec{Name: "  My World "})
	if err != nil {
		t.Fatal(err)
	}
	if world.Name != "My World" || world.GameMode != GameModeSurvival || world.Generator != GeneratorNormal || world.Difficulty != DifficultyNormal {
		t.Fatalf("defaults not applied: %+v", world)
	}
	got, err := store.Get(world.ID)
	if err != nil || got != world {
		t.Fatalf("reload mismatch: %+v %v", got, err)
	}
}

func TestCreateKeepsExplicitSeedIncludingZero(t *testing.T) {
	store := newTestStore(t)
	zero := int64(0)
	world, err := store.Create(Spec{Name: "a", Seed: &zero, Generator: "FLAT", GameMode: "creative", Difficulty: "hard"})
	if err != nil {
		t.Fatal(err)
	}
	if world.Seed != 0 || world.Generator != GeneratorFlat || world.GameMode != GameModeCreative || world.Difficulty != DifficultyHard {
		t.Fatalf("unexpected world %+v", world)
	}
}

func TestCreateRejectsInvalidSpecs(t *testing.T) {
	store := newTestStore(t)
	for _, spec := range []Spec{
		{Name: ""}, {Name: "  "}, {Name: "bad\x00name"}, {Name: string(make([]rune, 65))},
		{Name: "x", GameMode: "hardcore"}, {Name: "x", Generator: "void"}, {Name: "x", Difficulty: "brutal"},
	} {
		if _, err := store.Create(spec); !errors.Is(err, ErrInvalid) {
			t.Fatalf("spec %+v: want ErrInvalid, got %v", spec, err)
		}
	}
}

func TestListOrdersByLastPlayedAndSkipsJunk(t *testing.T) {
	store := newTestStore(t)
	base := time.Unix(1000, 0)
	store.now = func() time.Time { return base }
	older, _ := store.Create(Spec{Name: "older"})
	store.now = func() time.Time { return base.Add(time.Hour) }
	newer, _ := store.Create(Spec{Name: "newer"})
	if err := os.MkdirAll(filepath.Join(store.root, "not-an-id"), 0o700); err != nil {
		t.Fatal(err)
	}
	corrupt := filepath.Join(store.root, "0123456789abcdef")
	_ = os.MkdirAll(corrupt, 0o700)
	_ = os.WriteFile(filepath.Join(corrupt, metaFile), []byte("{"), 0o600)
	worlds, err := store.List()
	if err != nil || len(worlds) != 2 || worlds[0].ID != newer.ID || worlds[1].ID != older.ID {
		t.Fatalf("list = %+v, %v", worlds, err)
	}
	if err := store.Delete("0123456789abcdef"); err != nil {
		t.Fatalf("corrupt world must be deletable: %v", err)
	}
}

func TestRenameTouchDelete(t *testing.T) {
	store := newTestStore(t)
	world, _ := store.Create(Spec{Name: "a"})
	renamed, err := store.Rename(world.ID, " b ")
	if err != nil || renamed.Name != "b" {
		t.Fatalf("rename = %+v, %v", renamed, err)
	}
	if _, err := store.Rename(world.ID, ""); !errors.Is(err, ErrInvalid) {
		t.Fatalf("empty rename: %v", err)
	}
	store.now = func() time.Time { return time.Unix(5000, 0) }
	if err := store.Touch(world.ID); err != nil {
		t.Fatal(err)
	}
	if got, _ := store.Get(world.ID); got.LastPlayedUnix != 5000 {
		t.Fatalf("touch not persisted: %+v", got)
	}
	if err := store.Delete(world.ID); err != nil {
		t.Fatal(err)
	}
	if _, err := store.Get(world.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("after delete: %v", err)
	}
	if err := store.Delete(world.ID); !errors.Is(err, ErrNotFound) {
		t.Fatalf("double delete: %v", err)
	}
}

func TestMalformedIDsNeverReachTheFilesystem(t *testing.T) {
	store := newTestStore(t)
	for _, id := range []string{"", "..", "../x", "ABCDEF0123456789", "0123456789abcde"} {
		if _, err := store.Get(id); !errors.Is(err, ErrNotFound) {
			t.Fatalf("id %q: %v", id, err)
		}
		if err := store.Delete(id); !errors.Is(err, ErrNotFound) {
			t.Fatalf("delete id %q: %v", id, err)
		}
	}
}
