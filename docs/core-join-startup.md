# Core join startup evidence

The initial outbound sequence is `RequestChunkRadius`, `ServerboundLoadingScreen(Start)`,
then, after local loading completes, `ServerboundLoadingScreen(End)` and
`SetLocalPlayerAsInitialized`. Receiving `PlayerSpawn` alone does not complete loading.
These are control-flow findings from the reconstructed client, not a captured retail wire trace.

## Vanilla references

Lens references below use the source-backed raw view of artifact 6, version **1.26.50.26**.
`R:` references use the named 26.30 reconstruction under `reference/26.30/src/by-owner/`.

- **Radius first:** StartGame handler RVA `0x014de920` calls
  `LocalPlayer::requestChunkRadius`, RVA `0x04f2cb40`, directly. The latter constructs and
  sends the radius packet. Corroboration: `R:l/LegacyClientNetworkHandler.cpp:6673` and
  `R:l/LocalPlayer.cpp:1543`.
- **Loading transitions:** worker RVA `0x07112460` queues start/end packets when the loading
  screen changes state. Adapter RVA `0x07112d00` carries the embedded
  `ClientLoadingScreenSystemAnon::_updateLoadingScreenState` signature. Packet identity is
  confirmed by clone RVA `0x045bae10`, sharing vtable `0x1501b59b0`. This scheduled transition
  follows StartGame's direct radius request; end follows closing the loading screen.
- **Initialized send site:** RVA `0x04f2f4e0` checks loading state `0x10`, stable dimension
  state `0`, and a one-shot flag before sending the local runtime ID. Its packet vtable is
  `0x1501b5db0`, also identified by `TypeSchema<SetLocalPlayerAsInitializedPacket>` at
  RVA `0x0461af30`. Corroboration: `R:l/LocalPlayer.cpp:3339`, `:3370`, `:3392`, `:3421`.
  The named reconstruction resolves its additional client predicate (vtable slot `0x500`)
  to `isInWorldAndNotShowingAnyMenuScreens`, `R:c/ClientInstance.cpp:24930`.
  `R:c/ClientLoadingProgressTickingSystem.cpp:716` establishes the completed loading state;
  `:856` tests that state. Initialized is therefore a local readiness notification, after
  the loading screen closes, rather than a direct PlayerSpawn response.
- **Pack acquisition versus selection:** Lens stack handler RVA `0x014a9cf0` checks the
  selected stack's compatibility and required bit; incompatible required content takes
  fatal helper RVA `0x014acfa0`. Download-dialog RVA `0x05811a80` separately handles
  required/optional acquisition. The vanilla **1.26.50.4 resource pack**, read at
  `full/resource_pack/texts/en_US.lang:8242`–`:8245`, has distinct optional, required and
  server-required download prompts. `ui/progress_screen.json:1209` defines the world-loading
  screen independently. Neither the language strings nor UI JSON establish packet order.
- **Transfer destination:** Lens artifact 6, raw RVA `0x014c8e10`, identifies itself as the
  `ClientNetworkHandler::handle(..., TransferPacket const&)` handler and calls server-transfer
  initiator RVA `0x001899f0` on its ordinary destination path. The named reconstruction passes
  the packet's address and port to `WorldTransferInitiator::initiateTransferToServer` at
  `R:c/ClientNetworkHandler.cpp:16616`; `R:w/WorldTransferInitiator.cpp:4` builds that server
  connection. The vanilla pack's `texts/en_US.lang:1546` describes transfer as moving a player
  to another server, and `:8237` labels the world-transfer screen “Loading World”. These sources
  establish destination handoff, not a bridge shutdown algorithm or a spawn prerequisite.

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
