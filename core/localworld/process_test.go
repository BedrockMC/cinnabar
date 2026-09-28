package localworld

import (
	"bufio"
	"context"
	"os"
	"strings"
	"testing"
	"time"
)

const helperEnv = "LOCALWORLD_TEST_HELPER"

// TestMain doubles as the fake local server: it honours the ready/pause/resume/stop line protocol.
func TestMain(m *testing.M) {
	if mode := os.Getenv(helperEnv); mode != "" {
		if mode == "crash" {
			os.Exit(3)
		}
		if mode != "silent" {
			os.Stdout.WriteString("ready\n")
		}
		scanner := bufio.NewScanner(os.Stdin)
		for scanner.Scan() {
			if strings.TrimSpace(scanner.Text()) == "stop" {
				break
			}
		}
		os.Exit(0)
	}
	os.Exit(m.Run())
}

func testSpec() StartSpec {
	return StartSpec{World: World{ID: "0123456789abcdef", Name: "n", GameMode: "survival", Generator: "flat", Difficulty: "easy", Seed: -7}, Dir: os.TempDir()}
}

func TestProcessRunnerStartPauseStop(t *testing.T) {
	runner := ProcessRunner{Binary: os.Args[0], Env: []string{helperEnv + "=ok"}, StartTimeout: 10 * time.Second}
	inst, err := runner.Start(context.Background(), testSpec())
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(inst.Address(), "127.0.0.1:") {
		t.Fatalf("address = %q", inst.Address())
	}
	if err := inst.SetPaused(true); err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if err := inst.Stop(ctx); err != nil {
		t.Fatal(err)
	}
	select {
	case <-inst.Done():
	default:
		t.Fatal("Done not closed after Stop")
	}
}

func TestProcessRunnerReportsCrashDuringStartup(t *testing.T) {
	runner := ProcessRunner{Binary: os.Args[0], Env: []string{helperEnv + "=crash"}, StartTimeout: 10 * time.Second}
	if _, err := runner.Start(context.Background(), testSpec()); err == nil {
		t.Fatal("expected startup failure")
	}
}

func TestProcessRunnerTimesOutWhenNeverReady(t *testing.T) {
	runner := ProcessRunner{Binary: os.Args[0], Env: []string{helperEnv + "=silent"}, StartTimeout: 200 * time.Millisecond}
	if _, err := runner.Start(context.Background(), testSpec()); err == nil {
		t.Fatal("expected readiness timeout")
	}
}

func TestProcessRunnerRequiresBinary(t *testing.T) {
	if _, err := (ProcessRunner{}).Start(context.Background(), testSpec()); err == nil {
		t.Fatal("expected error")
	}
}
