//go:build darwin

package authcache

import (
	"bytes"
	"context"
	"errors"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"testing"

	"golang.org/x/oauth2"
)

func TestDerivedFixturesSupportPrivateReadsAndStableLeases(t *testing.T) {
	dir := derivedTestDir(t)
	path := filepath.Join(dir, "derived")
	canonical, err := canonicalizeCachePath(path)
	if err != nil || canonical != path {
		t.Fatalf("fixture path is not canonical: %v", err)
	}
	contents := []byte("synthetic fixture\n")
	if err := savePrivate(path, contents); err != nil {
		t.Fatal(err)
	}
	got, err := loadPrivate(path, maxCacheSize)
	if err != nil || !bytes.Equal(got, contents) {
		t.Fatalf("read private fixture: error = %v, contents match = %v", err, bytes.Equal(got, contents))
	}
	leasePath := path + ".lock"
	if _, err := leaseFileIdentity(leasePath); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("absent fixture lease error = %v, want missing", err)
	}
	if err := prepareLeasePath(leasePath); err != nil {
		t.Fatal(err)
	}
	before, err := leaseFileIdentity(leasePath)
	if err != nil {
		t.Fatal(err)
	}
	if err := prepareLeasePath(leasePath); err != nil {
		t.Fatal(err)
	}
	after, err := leaseFileIdentity(leasePath)
	if err != nil {
		t.Fatal(err)
	}
	if !os.SameFile(before, after) {
		t.Fatal("fixture lease identity changed on repeated preparation")
	}
}

func TestPersistentSourceCanonicalizesTrustedTopLevelAlias(t *testing.T) {
	raw := filepath.Join(t.TempDir(), "derived")
	canonical, err := canonicalizeCachePath(raw)
	if err != nil {
		t.Fatal(err)
	}
	if canonical == raw {
		t.Skip("temporary directory does not use a trusted top-level alias")
	}
	source := PersistentSource(context.Background(), raw, oauth2.StaticTokenSource(testOAuthToken("account-a")), io.Discard)
	persistent, ok := source.(*persistentAuthSource)
	if !ok {
		t.Fatal("persistent source was not constructed")
	}
	if persistent.path != canonical {
		t.Fatalf("persistent path = %q, want canonical path %q", persistent.path, canonical)
	}
}
