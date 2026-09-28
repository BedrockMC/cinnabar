package main

import (
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"runtime"

	"github.com/hashimthearab/rust-mcbe/core/localworld"
)

const localServerName = "bedrock-local-server"

func defaultLocalServerBinary() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	name := localServerName
	if runtime.GOOS == "windows" {
		name += ".exe"
	}
	return filepath.Join(filepath.Dir(exe), name), nil
}

// openLocalWorlds fails closed when the server binary is missing so an open never dies later with a vague error.
func openLocalWorlds(opts options, logger *slog.Logger) (*localworld.Manager, error) {
	binary := opts.localServerBin
	if binary == "" {
		var err error
		if binary, err = defaultLocalServerBinary(); err != nil {
			return nil, fmt.Errorf("locate local world server: %w", err)
		}
	}
	if info, err := os.Stat(binary); err != nil || info.IsDir() {
		return nil, fmt.Errorf("local world server binary not found at %s; build it with `make local-server`", binary)
	}
	store, err := localworld.OpenStore(opts.localWorldsDir)
	if err != nil {
		return nil, err
	}
	logger.Info("local worlds enabled", "dir", opts.localWorldsDir)
	return localworld.NewManager(store, localworld.ProcessRunner{Binary: binary, Log: logger}, logger), nil
}
