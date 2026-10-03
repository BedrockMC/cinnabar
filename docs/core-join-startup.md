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

## Near terrain can complete before distant requests

The presentation gate also accepts the loaded 3×3 neighborhood around the server's
player position once its resident sections have current light and acknowledged
meshes. It still requires a later GPU-completed frame. Far replies and their
lighting/meshing queues can continue after the loading screen closes. This avoids
requiring a fully drained view when the visible opaque count is below the dense
threshold. `fa7af5ed` connected the initialization notification to that older gate;
it is not proof that the BDS in the supplied screenshot withheld replies.

Vanilla references:

- Lens 26.30 `_calculateLoadingProgressView` (`0x1058f2920`) tests the spawn
  neighborhood when the full view is incomplete. Its initializer at `0x105940f20`
  builds nine `ChunkPos` offsets; the referenced data at `0x10deac070`,
  `0x10ddfaf20` and `0x10d945f70` plus the final pair cover x/z −1 through 1.
  Current 1.26.50.26 Lens loading notification at `0x07112460` was also inspected;
  it is the notification worker, not the neighborhood readiness calculation.
- `R:c/ClientLoadingProgressTickingSystem.cpp:666` checks those offsets;
  `:716` enters the completed state. `R:l/LocalPlayer.cpp:3339` and `:3421`
  connect loading completion to initialization notification.
- Vanilla pack `ui/progress_screen.json:1215` and
  `texts/en_US.lang:8153`, `:8179` supply the retained loading presentation.

`bds_local_startup_completes_with_distant_replies_withheld` runs real requests,
decoding, lighting, meshing and upload acknowledgements. Near terrain becomes
ready while a distant column remains withheld and the old drained predicate is
false (725 ms in the focused run). Releasing that reply still drains the stream.
`local_terrain_releases_after_a_gpu_frame_with_distant_work_pending` checks the
additional presentation fence. `bds_join_dense_columns_drain_with_a_stationary_camera`
checks full-height terrain; `bds_saved_terrain_drains_without_camera_motion` can
replay local occupancy records through `CINNABAR_BDS_TERRAIN`.

The full-height dense fixture drained in 5.71 s at the harness's normal 8 ms
frame cadence, with 495 visible meshes and no pending light or mesh jobs. The
accelerated 1 ms fixture previously exhausted its frame count under concurrent
gate load; the convergence test now keeps wall time closer to its simulated
reply clock. Earlier accelerated runs took 7.37 s before and 6.41 s afterward;
these mixed-load observations are not a pipeline speedup claim. An occupancy
replay extracted read-only from the restored BDS world drained in 1.25–2.94 s. Neither reproduces the reported minutes
or 7 FPS, and occupancy is not an exact packet capture. The restored client-tail
log is empty. These offline results do not establish a live BDS join time.
