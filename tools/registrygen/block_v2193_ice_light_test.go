package main

import "testing"

// Current IceBlock 071305c0 and FrostedIceBlock 08723f10 both use the
// inherited 0365cdf0 getter: dampening zero, including every frosted age.
func TestIceNativeLightDampening(t *testing.T) {
	t.Parallel()
	cases := []struct {
		name, state string
		filter      byte
	}{
		{name: "minecraft:ice", state: `{}`, filter: 0},
		{name: "minecraft:frosted_ice", state: `{"age":{"type":"int","value":0}}`, filter: 0},
		{name: "minecraft:frosted_ice", state: `{"age":{"type":"int","value":1}}`, filter: 0},
		{name: "minecraft:frosted_ice", state: `{"age":{"type":"int","value":2}}`, filter: 0},
		{name: "minecraft:frosted_ice", state: `{"age":{"type":"int","value":3}}`, filter: 0},
		{name: "minecraft:packed_ice", state: `{}`, filter: 15},
		{name: "minecraft:blue_ice", state: `{}`, filter: 15},
		{name: "test:ice", state: `{}`, filter: 15},
	}
	for _, c := range cases {
		t.Run(c.name+c.state, func(t *testing.T) {
			t.Parallel()
			records := []Record{{Name: c.name, StateJSON: []byte(c.state)}}
			properties := []byte{15 << 4}
			// Even a conflicting external table must not override identified
			// native values. Unrelated known blocks must remain unchanged.
			retail := map[string]PMMPLightProperties{c.name: {Opacity: 1}}
			changed, err := applyRetailLightCorrections(records, properties, retail)
			if err != nil {
				t.Fatal(err)
			}
			if got := properties[0]; got != c.filter<<4 {
				t.Fatalf("light = %#x, want %#x", got, c.filter<<4)
			}
			wantChanged := 0
			if c.filter != 15 {
				wantChanged = 1
			}
			if changed != wantChanged {
				t.Fatalf("changed = %d, want %d", changed, wantChanged)
			}
			changed, err = applyRetailLightCorrections(records, properties, retail)
			if err != nil || changed != 0 {
				t.Fatalf("repeat correction changed=%d err=%v", changed, err)
			}
		})
	}
}
