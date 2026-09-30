package lockfile

import (
	"bufio"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

func TestAcquireExistingDoesNotCreateMissingParent(t *testing.T) {
	parent := filepath.Join(t.TempDir(), "missing")
	lease, err := AcquireExisting(filepath.Join(parent, "lease"), 0)
	if lease != nil {
		_ = lease.Close()
		t.Fatal("AcquireExisting returned a lease for a missing parent")
	}
	if !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("AcquireExisting error = %v, want missing", err)
	}
	if _, err := os.Lstat(parent); !errors.Is(err, fs.ErrNotExist) {
		t.Fatalf("AcquireExisting created its missing parent: %v", err)
	}
}

func TestAcquireStillCreatesMissingParentAndFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "missing", "lease")
	lease, err := Acquire(path, 0)
	if err != nil {
		t.Fatal(err)
	}
	if err := lease.Close(); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Lstat(path); err != nil || !info.Mode().IsRegular() {
		t.Fatalf("Acquire did not create a regular lease file: info=%v error=%v", info, err)
	}
}

// The auth-cache lease is an existing lock file; a holder that dies without releasing it (the
// kernel drops the lock with its descriptors) must not block the next core.
func TestLeaseOfADeadHolderIsFree(t *testing.T) {
	if path := os.Getenv("LOCKFILE_HOLDER"); path != "" {
		if _, err := AcquireExisting(path, 0); err != nil {
			os.Exit(2)
		}
		fmt.Println("held")
		time.Sleep(time.Minute)
		os.Exit(3)
	}
	path := filepath.Join(t.TempDir(), "lease")
	if err := os.WriteFile(path, nil, 0o600); err != nil {
		t.Fatal(err)
	}
	holder := exec.Command(os.Args[0], "-test.run=^TestLeaseOfADeadHolderIsFree$")
	holder.Env = append(os.Environ(), "LOCKFILE_HOLDER="+path)
	stdout, err := holder.StdoutPipe()
	if err != nil {
		t.Fatal(err)
	}
	if err := holder.Start(); err != nil {
		t.Fatal(err)
	}
	defer func() { _ = holder.Process.Kill(); _ = holder.Wait() }()
	if line, _ := bufio.NewReader(stdout).ReadString('\n'); line != "held\n" {
		t.Fatalf("holder reported %q", line)
	}
	if _, err := AcquireExisting(path, 0); !errors.Is(err, ErrBusy) {
		t.Fatalf("AcquireExisting beside a live holder = %v, want ErrBusy", err)
	}
	_ = holder.Process.Kill()
	_ = holder.Wait()
	lease, err := AcquireExisting(path, 0)
	if err != nil {
		t.Fatalf("AcquireExisting after the holder died = %v", err)
	}
	_ = lease.Close()
}
