package control

import (
	"errors"
	"strings"
	"testing"
)

// Logged service errors keep their status but never a token.
func TestRedactErrorHidesTokens(t *testing.T) {
	err := errors.New(`POST gatherings: 401 Unauthorized: {"token":"abc123","Authorization":"MCToken eyJhbGciOi.eyJzdWIi.c2ln"} XBL3.0 x=123;eyJraWQ`)
	got := RedactError(err)
	for _, secret := range []string{"abc123", "eyJhbGciOi", "x=123;"} {
		if strings.Contains(got, secret) {
			t.Fatalf("redacted error %q still holds %q", got, secret)
		}
	}
	if !strings.Contains(got, "401 Unauthorized") {
		t.Fatalf("redacted error %q lost its status", got)
	}
}
