package main

import "testing"

// Only an unimplemented-block default is replaced; known values and unlisted names stay.
func TestRetailLightCorrectionsReplaceOnlyUnimplementedDefaults(t *testing.T) {
	records := []Record{
		{Name: "minecraft:glow_lichen"},
		{Name: "minecraft:vine"},
		{Name: "minecraft:stone"},
		{Name: "minecraft:torch"},
		{Name: "test:unlisted"},
	}
	properties := []byte{15 << 4, 15 << 4, 15 << 4, 14, 15 << 4}
	retail := map[string]PMMPLightProperties{
		"minecraft:glow_lichen": {Brightness: 7, Opacity: 0},
		"minecraft:vine":        {Brightness: 0, Opacity: 0},
		"minecraft:stone":       {Brightness: 0, Opacity: 1},
		"minecraft:torch":       {Brightness: 0, Opacity: 0},
	}
	changed, err := applyRetailLightCorrections(records, properties, retail)
	if err != nil {
		t.Fatal(err)
	}
	want := []byte{7, 0, 15 << 4, 14, 15 << 4}
	for index := range want {
		if properties[index] != want[index] {
			t.Fatalf("state %d = %#x, want %#x", index, properties[index], want[index])
		}
	}
	if changed != 2 {
		t.Fatalf("changed = %d", changed)
	}
}
