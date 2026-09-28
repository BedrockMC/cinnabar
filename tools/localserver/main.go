// Command bedrock-local-server hosts one saved single-player world on dragonfly for the core.
// It prints "ready" once listening and reads "pause", "resume" and "stop" lines on stdin;
// stdin EOF and SIGINT/SIGTERM also stop it.
package main

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"log/slog"
	"os"
	"os/signal"
	"strings"
	"syscall"

	"github.com/df-mc/dragonfly/server/world"
)

const defaultTickRange = 6 // dragonfly's default; the API has no getter to restore from

func main() {
	if err := run(os.Args[1:], os.Stdin, os.Stdout, os.Stderr); err != nil {
		fmt.Fprintln(os.Stderr, "bedrock-local-server:", err)
		os.Exit(1)
	}
}

func run(args []string, stdin io.Reader, stdout, stderr io.Writer) error {
	cfg, err := parseSettings(args, stderr)
	if err != nil {
		return err
	}
	logger := slog.New(slog.NewTextHandler(stderr, nil))
	conf, err := cfg.userConfig().Config(logger)
	if err != nil {
		return fmt.Errorf("configure server: %w", err)
	}
	conf.Generator = cfg.dimensionGenerator
	srv := conf.New()
	worlds := []*world.World{srv.World(), srv.Nether(), srv.End()}
	cfg.applyTo(worlds...)
	srv.Listen()
	accepting := make(chan struct{})
	go func() {
		defer close(accepting)
		for range srv.Accept() {
		}
	}()
	fmt.Fprintln(stdout, "ready")

	ctx, stop := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer stop()
	serveCommands(ctx, stdin, func(paused bool) { setPaused(worlds, paused) })
	closeErr := srv.Close()
	<-accepting
	return closeErr
}

// serveCommands runs the stdin protocol until "stop", EOF or ctx ends.
func serveCommands(ctx context.Context, stdin io.Reader, pause func(bool)) {
	lines := make(chan string)
	go func() {
		defer close(lines)
		scanner := bufio.NewScanner(stdin)
		for scanner.Scan() {
			lines <- strings.TrimSpace(scanner.Text())
		}
	}()
	for {
		select {
		case <-ctx.Done():
			return
		case line, ok := <-lines:
			if !ok || line == "stop" {
				return
			}
			switch line {
			case "pause":
				pause(true)
			case "resume":
				pause(false)
			}
		}
	}
}

// setPaused freezes time and block/entity ticking; connected players stay connected.
func setPaused(worlds []*world.World, paused bool) {
	for _, w := range worlds {
		if paused {
			w.SetTickRange(0)
			w.StopTime()
		} else {
			w.SetTickRange(defaultTickRange)
			w.StartTime()
		}
	}
}
