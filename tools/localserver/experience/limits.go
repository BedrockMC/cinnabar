package experience

import "time"

// dataQuota is the total number of private data bytes one Experience may store.
const dataQuota = 16 << 20

// flushInterval is how often RunFlusher writes dirty Experience data to disk.
const flushInterval = 5 * time.Second

// loadDeadline bounds how long a helper may take to answer load, compilation included.
const loadDeadline = 30 * time.Second

// resultDeadline bounds how long a helper may take to answer one callback.
const resultDeadline = 2 * time.Second

// maxRestarts is how many helper restarts restartWindow may hold; one more quarantines the
// Experience.
const maxRestarts = 3

// restartWindow is the span over which helper restarts are counted.
const restartWindow = 5 * time.Minute

// strikeLimit is how many strikes within strikeWindow quarantine the Experience.
const strikeLimit = 3

// strikeWindow is the span over which strikes are counted.
const strikeWindow = time.Minute

// shutdownGrace is how long a helper may take to exit after the shutdown frame before it is
// killed.
const shutdownGrace = time.Second

// maxFrameBytes is the largest frame body. It must equal the Rust runtime's MAX_FRAME_BYTES,
// which TestFrameLimitMatchesRust checks against the limits fixture.
const maxFrameBytes = 1 << 20

// stderrLineBytes is the longest helper stderr line that is logged; the rest of a longer line is
// dropped.
const stderrLineBytes = 4 << 10
