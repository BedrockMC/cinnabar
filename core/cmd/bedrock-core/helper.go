package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"runtime"

	"github.com/hashimthearab/rust-mcbe/core/update"
)

// Injected at release build time via -ldflags "-X main.releaseVersion=... -X main.trustedUpdateKeys=id:b64,...".
var (
	releaseVersion    = "0.0.0-dev"
	trustedUpdateKeys = ""
)

// helperMode reports whether args select a client-driven helper subcommand and, if so, runs it.
func helperMode(ctx context.Context, args []string, stdout, stderr io.Writer) (bool, int) {
	if len(args) == 0 || args[0] != "check-update" {
		return false, 0
	}
	if err := runCheckUpdate(ctx, args[1:], stdout); err != nil {
		fmt.Fprintf(stderr, "%s: %v\n", args[0], err)
		return true, 1
	}
	return true, 0
}

func runCheckUpdate(ctx context.Context, args []string, stdout io.Writer) error {
	flags := flag.NewFlagSet("check-update", flag.ContinueOnError)
	manifestURL := flags.String("manifest-url", "", "signed manifest URL")
	channel := flags.String("channel", "stable", "release channel")
	platform := flags.String("platform", runtime.GOOS+"-"+runtime.GOARCH, "artifact platform key")
	current := flags.String("current", releaseVersion, "running client version")
	if err := flags.Parse(args); err != nil {
		return err
	}
	keys, err := update.ParseKeys(trustedUpdateKeys)
	if err != nil {
		return err
	}
	if len(keys) == 0 {
		return errors.New("this build has no trusted update keys")
	}
	result, err := update.Check(ctx, update.Config{
		ManifestURL: *manifestURL, Channel: *channel, Platform: *platform, Current: *current, Keys: keys,
	})
	if err != nil {
		return err
	}
	return json.NewEncoder(stdout).Encode(result)
}
