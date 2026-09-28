package main

import (
	"bytes"
	"context"
	"testing"
)

func TestHelperModeIgnoresProxyArgs(t *testing.T) {
	if handled, _ := helperMode(context.Background(), []string{"-upstream", "x:1"}, &bytes.Buffer{}, &bytes.Buffer{}); handled {
		t.Fatal("proxy arguments must not select helper mode")
	}
}

func TestCheckUpdateRequiresTrustedKeys(t *testing.T) {
	var stderr bytes.Buffer
	handled, code := helperMode(context.Background(), []string{"check-update", "-manifest-url", "https://example.test/m"}, &bytes.Buffer{}, &stderr)
	if !handled || code != 1 {
		t.Fatalf("handled=%v code=%d stderr=%s", handled, code, stderr.String())
	}
}

func TestUploadCrashRequiresDSN(t *testing.T) {
	t.Setenv("CINNABAR_SENTRY_DSN", "")
	if handled, code := helperMode(context.Background(), []string{"upload-crash", "-file", "x"}, &bytes.Buffer{}, &bytes.Buffer{}); !handled || code != 1 {
		t.Fatalf("handled=%v code=%d", handled, code)
	}
}
