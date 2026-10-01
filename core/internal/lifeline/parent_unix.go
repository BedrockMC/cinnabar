//go:build !windows

package lifeline

import (
	"os"
	"time"
)

// WatchParent returns a channel closed once parent is no longer this process's parent (it died
// and the core was reparented to launchd, init or a subreaper), including before the call.
func WatchParent(parent int, interval time.Duration) <-chan struct{} {
	gone := make(chan struct{})
	go func() {
		ticker := time.NewTicker(interval)
		defer ticker.Stop()
		for os.Getppid() == parent {
			<-ticker.C
		}
		close(gone)
	}()
	return gone
}
