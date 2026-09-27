package main

import (
	"reflect"
	"strings"
	"testing"
)

func TestBuildJoinsHardnessWithHarvestClasses(t *testing.T) {
	pmmp := []byte(`{
		"minecraft:obsidian": {"hardness": 35.0},
		"minecraft:grass_block": {"hardness": 0.6000000238418579},
		"minecraft:web": {"hardness": 4.0},
		"minecraft:custom_only": {"hardness": 1.0},
		"minecraft:cocoa": {"hardness": 0.2},
		"minecraft:glass": {"hardness": 0.3}
	}`)
	prismarine := []byte(`[
		{"name": "obsidian", "material": "incorrect_for_wooden_tool", "harvestTools": {"966": true, "971": true}},
		{"name": "grass_block", "material": "mineable/shovel"},
		{"name": "web", "material": "coweb", "harvestTools": {"939": true, "1134": true}},
		{"name": "cocoa", "material": "plant;mineable/axe"},
		{"name": "glass", "material": "default"}
	]`)
	rows, err := build(pmmp, prismarine)
	if err != nil {
		t.Fatal(err)
	}
	// Rows without tool evidence (absent, hand-only material, or an unverified
	// material) are unresolved rather than hand-harvestable.
	want := []row{
		{"minecraft:cocoa", 0.2, []string{}, []string{}, -1, true},
		{"minecraft:custom_only", 1, nil, nil, -1, true},
		{"minecraft:glass", 0.3, []string{}, []string{}, -1, true},
		{"minecraft:grass_block", 0.6, []string{"shovel"}, []string{}, -1, false},
		{"minecraft:obsidian", 35, []string{"pickaxe"}, []string{"pickaxe"}, 3, false},
		{"minecraft:web", 4, []string{"shears", "sword"}, []string{"shears", "sword"}, 0, false},
	}
	if !reflect.DeepEqual(rows, want) {
		t.Fatalf("rows = %#v\nwant %#v", rows, want)
	}
}

func TestBuildRejectsUnknownHarvestToolIDs(t *testing.T) {
	_, err := build([]byte(`{"minecraft:stone": {"hardness": 1.5}}`),
		[]byte(`[{"name": "stone", "material": "mineable/pickaxe", "harvestTools": {"1": true}}]`))
	if err == nil || !strings.Contains(err.Error(), "unclassified harvest tool id 1") {
		t.Fatalf("err = %v", err)
	}
}

func TestBuildRejectsMissingHardness(t *testing.T) {
	_, err := build([]byte(`{"minecraft:stone": {}}`), []byte(`[]`))
	if err == nil {
		t.Fatal("missing hardness must fail closed")
	}
}
