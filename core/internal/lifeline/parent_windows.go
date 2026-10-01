//go:build windows

package lifeline

import (
	"os"
	"time"
)

// WatchParent returns a channel closed once parent exits; Windows never reparents, so this waits
// on a process handle instead of polling.
func WatchParent(parent int, _ time.Duration) <-chan struct{} {
	gone := make(chan struct{})
	process, err := os.FindProcess(parent)
	if err != nil {
		close(gone) // already exited
		return gone
	}
	go func() {
		_, _ = process.Wait()
		close(gone)
	}()
	return gone
}
