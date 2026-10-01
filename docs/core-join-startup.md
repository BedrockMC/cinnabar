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
not send duplicates. The transfer case sends StartGame then Transfer and closes without
ever supplying spawn prerequisites. The destination reaches Rust as a typed terminal event,
which the app routes to its existing reconnect owner.

This verifies the startup packet contract. It does not certify all terrain-readiness
thresholds, dimension transitions, consent dialogs, or live-server/visual parity. The existing
terrain presentation gate remains the app's readiness criterion; full parity remains open.
