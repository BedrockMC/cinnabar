# Core join startup evidence


## Vanilla references

Lens references below use the source-backed raw view of artifact 6, version **1.26.50.26**.
`R:` references use the named 26.30 reconstruction under `reference/26.30/src/by-owner/`.


## Implementation and regression coverage

The Rust login sends radius before loading-start and returns the stream once the server's
spawn prerequisites arrive. It retains a pending runtime ID. The app validates and installs
packs, waits for its terrain presentation gate, closes its loading screen, then queues one
completion command. That command sends loading-end followed by initialized once.

`offline_core_preserves_spawn_order_and_startup_transfer` starts the production Go relay
against a local scripted upstream and runs the actual Rust socket login. The upstream
asserts every relevant outbound packet in order, with a round-trip barrier proving that
completion cannot precede explicit presentation readiness. A second completion call must
not send duplicates. The transfer cases send StartGame then Transfer, in separate frames or
one batch, and immediately close without supplying spawn prerequisites. A connection-context
barrier waits for upstream EOF before starting either relay pump; no sleep controls the race.
The destination reaches Rust as a typed terminal event, which the app routes to its existing
reconnect owner.

The relay drains queued upstream batches before teardown when the reverse writer observes
an ordinary upstream close. Rust also reads terminal input if its startup response write
fails: Transfer or Disconnect takes precedence over that write failure. Without either
terminal packet it retains the original write error; the existing startup deadline and
owner cancellation still bound the drain. Scripted tests force write failure with both batch
layouts, and relay tests cover queued delivery and cancellation.

This verifies the startup packet contract. It does not certify all terrain-readiness
thresholds, dimension transitions, consent dialogs, or live-server/visual parity. The existing
terrain presentation gate remains the app's readiness criterion; full parity remains open.
