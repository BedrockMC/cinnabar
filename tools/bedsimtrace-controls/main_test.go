package main

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

func TestControlsEmissionIsRepeatableAndRetainsDistinctInputSemantics(t *testing.T) {
	var first, second bytes.Buffer
	if err := emit(&first); err != nil {
		t.Fatal(err)
	}
	if err := emit(&second); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(first.Bytes(), second.Bytes()) {
		t.Fatal("nonrepeatable control oracle")
	}
	decoder := json.NewDecoder(&first)
	var records []record
	for decoder.More() {
		var next record
		if err := decoder.Decode(&next); err != nil {
			t.Fatal(err)
		}
		records = append(records, next)
	}
	if len(records) != 14 {
		t.Fatalf("unexpected record count: %d", len(records))
	}
	if records[1].Processed == records[3].Processed {
		t.Fatal("processed clamp and raw scaling collapsed")
	}
	if records[6].Processed != [2]float32{} {
		t.Fatal("explicit zero ignored")
	}
	if records[8].Processed != records[2].Processed {
		t.Fatal("explicit one did not override consumption")
	}
}

func TestPinnedFixtureEqualsExecutedOracle(t *testing.T) {
	fixture, err := os.ReadFile("../../crates/sim/fixtures/bedsim-34d11dc5-controls.jsonl")
	if err != nil {
		t.Fatal(err)
	}
	var actual bytes.Buffer
	if err := emit(&actual); err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(fixture, actual.Bytes()) {
		t.Fatal("pinned controls fixture differs from exact-main execution")
	}
}
