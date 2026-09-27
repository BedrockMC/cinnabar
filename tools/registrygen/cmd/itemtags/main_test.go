package main

import (
	"strings"
	"testing"
)

func TestCompactSortsTagsAndItems(t *testing.T) {
	table, err := compact([]byte(`{"minecraft:b":["minecraft:z","minecraft:y"],"minecraft:a":["minecraft:x"]}`))
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(table), "\n")
	if len(lines) != 3 || lines[1] != "minecraft:a\tminecraft:x" || lines[2] != "minecraft:b\tminecraft:y minecraft:z" {
		t.Fatalf("table = %q", table)
	}
}

func TestCompactRejectsDuplicateOrInvalidItems(t *testing.T) {
	for _, input := range []string{
		`{"minecraft:a":["minecraft:x","minecraft:x"]}`,
		`{"minecraft:a":["Not An Id"]}`,
	} {
		if _, err := compact([]byte(input)); err == nil {
			t.Fatalf("compact(%s) succeeded", input)
		}
	}
}
