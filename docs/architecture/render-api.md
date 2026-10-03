# Render contracts: restructuring step 1

`render-api` owns the contracts shared by world publication, skin admission and
rendering. It uses only the Rust standard library. `render` no longer depends on
`client-world` or its protocol stack.

| Contract | Previous definition | New definition |
| --- | --- | --- |
| `PublicationAllowance`, `PublicationPermit`, `PublicationPermitStage`, `PublicationServiceConfig` | `client-world/src/publication_config.rs` | `render-api/src/publication.rs` |
| `CLASSIC_SKIN_SIDE`, `MAX_CLASSIC_SKIN_SIDE`, `expand_legacy_skin_rgba8` | `protocol/src/actor/skin/legacy.rs` | `render-api/src/skin.rs` |
| `MAX_STANDARD_SKIN_SIDE` | `protocol/src/actor.rs` | `render-api/src/skin.rs` |
| `MAX_SKIN_ANIMATION_LAYERS` | `protocol/src/actor/skin/animation.rs` | `render-api/src/skin.rs` |

The implementations and values are unchanged. The ten permit tests move with the
implementation. Existing protocol and render tests still cover skin admission,
legacy limb conversion, texture sizing and permit transfer through GPU publication.
Protocol retains packet normalization and skin animation types; client-world retains
world state and scheduling. Both keep their existing public re-exports so app callers
use the same Rust types without adapters or duplicate definitions.

The dependency rules in `tools/architecture/policy.toml` register the new crate,
allow client-world and protocol to use it, and replace render's client-world edge
with render-api. `dependency_free` rejects every dependency kind on render-api,
including dev, build and target-specific dependencies. Render's transitive ban on
client-world and protocol follows local production/build edges, starting from all of
render's dependency kinds. It resolves renamed and workspace-inherited dependencies
and rejects unregistered local paths encountered while checking the boundary.

## Preserved behavior references

This is a code ownership change, not a new parity claim. The existing skin behavior
has these references:

- **R:SkinValidator:21**, **R:SkinValidator:59**, **R:SkinValidator:107** in the
  26.30 reconstruction's `by-owner/s/SkinValidator--90d17d7a7f20.cpp`: classic
  image sizes, half-height expansion and mirrored limb pixels. Local source root:
  `~/coding/go/lunar/refs/mcsrc-1.26.50/reference/26.30/src`.
- **Lens 1.26.50.26**, artifact 6, RVA `0x4c6a8e0`,
  `ClientSkinSystem::_initializeClientSkin`, raw source-backed lines 538/543:
  the `animated_32x32` and `animated_face` animation geometry slots. Located with
  `source_search` for `animated_32x32`, then inspected with `artifact_function`;
  the response reports `source_backed: true` and `derived_source: true`.
- The installed vanilla pack at
  `.local/assets/bedrock-samples/v1.26.50.4/full/resource_pack/`:
  `render_controllers/persona.render_controllers.json:4,21,38` and `:55,88,121`
  defines face, 32x32 and 128x128 slots for first- and third-person rendering.

The publication pacing bounds and the 512-pixel admission ceiling remain existing
Cinnabar policy. They are not newly attributed to vanilla. No source or pack assets
are copied into this change.
