# Client presentation and optional acceptance

Step 5 moves actor publication, equipment, viewmodel fallback, camera and audio
implementation into `client-presentation`. The app composes its plugins and keeps
explicit scheduling. It supplies borrowed observations from the owners that step 4
extracts separately; this change does not move movement, item use, mining, combat,
reconciliation, network transport or connection startup.

## Ownership and frame order

`ClientPresentationPlugin` owns retained actor preparation state.
`CameraPresentationPlugin` owns camera resources, and `AudioPresentationPlugin`
owns retained sound settings and cue state. The app's systems adapt existing
resources to these plugins at the same points in the frame schedule.

Actor preparation still precedes outbound interactions; actor publication still
follows network send. Preparation borrows the stream and the exact immutable pack
and item snapshots. It synchronizes the local pose, samples actor world state when
a tick advances, advances interpolation, selects rigs and equipment, and captures
the batch for publication. Neither the queue ownership nor publication permits
change. Item-use animation observations are sampled after actor interpolation, at
the same boundary as before. Local equipment pose flags are refreshed after local
pose synchronization instead of being retained from an earlier frame.

Presentation receives physics and collision queries through read-only interfaces.
The viewmodel receives a borrowed stream, immutable artwork, a screen visibility
flag and the current interaction authority identity. These interfaces do not own
or mutate gameplay state. The host retains the original cursor and system ordering.

`assets::SessionEntityPack` owns the immutable compiled entity/artwork contract.
The shared pinned content helpers also live in `assets`, so startup, metrics and
viewmodel validation consume the same registry bytes and manifest identity.
The presentation crate owns prepared actor artwork and item presentation bundles;
pack compilation and session admission remain outside it. Original asset identity
and pointer-equality checks continue to fence prepared artwork across reloads.

## Evidence and optional builds

`diagnostics` owns ordinary metrics and shared marker names. The optional
`acceptance` crate owns acceptance trackers, proofs, witness parsing and evidence
formatting. App adapters provide observations and apply returned commands at the
existing runtime boundaries. The model witness does not own the client world or
session transport.

The app's `acceptance` feature preserves the existing evidence-enabled build.
A build with `--no-default-features` excludes the acceptance dependency. Normal
telemetry does not depend on the optional evidence crate.

## Enforced boundaries

The architecture policy rejects direct, renamed and transitive dependencies from
presentation to app, runtime, gameplay, session transport or acceptance. Acceptance
and diagnostics cannot depend on app or the gameplay/session implementations.
Module checks reject rooted app imports and owned world/player authority inside
the plugins. Presentation test support cannot be enabled by a production dependency.
Regression fixtures cover permitted borrowed observations, forbidden ownership,
renamed dependencies and hidden transitive edges. Existing marker ownership checks
follow the relocated producers.

## References and validation scope

This change relocates existing behavior; it does not close a vanilla parity gate.
The first-person presentation source retains its current-client Lens annotations,
including `0x04fa7e50` for the camera stack, `0x04f9a2e0` for icon placement,
`0x04f9e2e0` for the offhand route, and `0x14ffa90e0` for texel conversion.
Source-first Lens searches during this migration found the older reconstructed
`ItemInHandRenderer` entries. A search restricted to 1.26.50.26 for
`renderFirstPerson` returned no source, and the current `0x04fa7e50` function read
reported source unavailable. No new current-client claim relies on that read.

The inspected 26.30 by-owner reconstruction identifies the corresponding
first-person function at `R:ItemInHandRenderer:15007`, offhand function at
`R:ItemInHandRenderer:7687`, and texture tessellation at
`R:TextureTessellator:116`. The retained audio routes correspond to
`R:SoundEngine:1545` and `R:SoundEngine:1606`. The retained spyglass look path cites
`R:LocalPlayer:5165`. The installed vanilla pack's
`render_controllers/player.render_controllers.json:4` defines first-person arm
visibility from both held items; that remains the equipment model's reference.
The installed pack was read through the worktree's `.local` symlink; no carrier or
installed asset was written.

Validation uses local compilation, affected tests and the architecture gate.
No client session or live server is started. See
[actor-edit rebuild measurements](../evidence/client-presentation-build-timings.md).
