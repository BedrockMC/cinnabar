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
		if mode == "docker" {
			runFakeDocker(os.Args[1:])
		}
		if mode == "crash" {
			os.Exit(3)
		}
		switch mode {
		case "silent":
		case "bds":
			os.Stdout.WriteString("[INFO] Starting Server\r\n[INFO] Server started.\r\n")
		default:
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

const dockerLogEnv = "LOCALWORLD_TEST_DOCKER_LOG"

// runFakeDocker records each invocation and answers the subcommands the container runner uses.
func runFakeDocker(args []string) {
	logPath := os.Getenv(dockerLogEnv)
	if f, err := os.OpenFile(logPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o600); err == nil {
		f.WriteString(strings.Join(args, " ") + "\n")
		f.Close()
	}
	exists := func(suffix string) bool { _, err := os.Stat(logPath + suffix); return err == nil }
	touch := func(suffix string) { _ = os.WriteFile(logPath+suffix, nil, 0o600) }
	switch {
	case len(args) == 0:
		os.Exit(2)
	case args[0] == "info":
		if os.Getenv("LOCALWORLD_TEST_DOCKER_DOWN") != "" {
			os.Exit(1)
		}
	case args[0] == "image":
		if !exists(".pulled") {
			os.Exit(1)
		}
	case args[0] == "pull":
		touch(".pulled")
	case args[0] == "stop":
		touch(".stop")
	case args[0] == "run":
		os.Stdout.WriteString("[INFO] Server started.\n")
		for i := 0; i < 1500 && !exists(".stop"); i++ {
			time.Sleep(20 * time.Millisecond)
		}
	}
	os.Exit(0)
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
