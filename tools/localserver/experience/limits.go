package experience

import "time"

// dataQuota is the total number of private data bytes one Experience may store.
const dataQuota = 16 << 20

// flushInterval is how often RunFlusher writes dirty Experience data to disk.
const flushInterval = 5 * time.Second
