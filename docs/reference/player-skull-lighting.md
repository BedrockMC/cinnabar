# Placed skull lighting

The placed-head brightness correction uses the target-family current client
reconstruction, with the matching executable as the authority for dispatch,
material identity and light-coordinate constants. The native installed client
is a near-version material/shader witness, rather than an identical-version
acceptance artifact. Game and pack targets remain defined by
`assets/bedrock-target.json` and `assets/vanilla-source.json`.

## Current-client evidence

MCSRC `current/1.26.50.26`, matching executable SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`:

| RVA | Observed contract |
| --- | --- |
| `07bf0820` | Skull renderer constructor installs the vtable whose placed-render slot points to `07bf4760`. |
| `07bf4760` | Placed render supplies the skull's integer block position to light setup `0213efc0`. |
| `0213efc0` | Reads `BlockSource::getLightColor` through `0319f150`, with minimum block light zero. |
| `0213ed00` | Divides the two retained brightness levels by 16, samples `LightTexture::getColorForUV` (`07083ac0`), and publishes RGB `TILE_LIGHT_COLOR`. The matched PE verifies the divisor. |
| `07c04e10` | Head-model constructor selects `mob_head.skinning`; the matching executable verifies the material string and its length. |
| `0148c360` / `01e889d0` | Piglin and dragon head constructors select `mob_head.skinning` and `dragon_head.skinning`, respectively. Both inherit the ordinary alpha-tested entity material. |
| `07bf4760` / `07bf4d70` | Uses the backing block's type identity to select the model, material and texture, then submits through the block-actor model renderer. Player head selects model `+0x9f0`. |
| `0de28e0a` / `0e000300` | Registers `player_head` as SkullBlock. Six companion registrations install the other current head types. |
| `0bbbc910` / `0319f150` / `05d7d290` | SkullBlock sets light filter zero. Light setup reads the requested cell's retained nibbles directly, without choosing neighbouring cells. |
| `0ab69990` | The network light component decoder looks up `lightLevel` inside the component compound and retains its byte value. |
| `0ab6b270` | The light emission decoder retains the distinct nested byte field `emission`. Matching serializer `0ab6b110` uses the same name. |

The current exported bodies are in `src/__unmapped/02.cpp`, `07.cpp` and `0a.cpp`;
function provenance is in `index/functions`. Recovered owner names are
navigation aids. No reconstructed bodies or proprietary assets are shipped.

The installed `1.26.51.01` vanilla material chain is
`mob_head:entity_alphatest`, with point sampling, no culling and an alpha cutoff
of 0.5. Its ordinary Fancy entity shader uses posed world normals, the same
native shading polynomial represented by `render_api::ACTOR_SHADE_COEFFICIENTS`,
and gamma-domain texture/light products. Existing actor lightmap witnesses
establish byte quantization and clamp-linear lookup at the /16 coordinates.

## Zeno custom heads

The affected Zeno Practice lobby heads are server-defined blocks, rather than
vanilla Skull block actors. A read-only authenticated capture from
`zenomc.org:19132` contains 480 custom head definitions. Their components use
`minecraft:light_dampening: {lightLevel: 0}`; their material instances enable
ambient occlusion and face dimming. Nearby live actor tracing found only the
local player and no Skull block-actor NBT at these heads.

The protocol decoder previously looked for a number on the component itself.
It discarded the compound's zero, then the overlay applied its omitted-component
default filter of 15. This incorrectly removed skylight inside every custom
head, making the inset model faces dark. The decoder now retains the nested
network `lightLevel` value for dampening and `emission` for emission. Existing
scalar definitions remain supported; odd non-finite values are skipped. No lighting multiplier,
minimum-light override or material-flag change is involved.

Network-NBT regressions exercise zero dampening and nonzero emission. The
overlay regression verifies that explicit zero survives compilation while an
omitted dampening component still defaults to 15.

The final Rust client build succeeded with executable SHA-256
`298bf9416a0d0efba1cc7b20b6e254d10f9c0b03a4d27983096b6556f272ab18`.
The user tested the rebuilt client on Zeno Practice on macOS/Metal with ordinary
controls and confirmed that the heads render perfectly. All seven explicitly
run Metal shader/readback tests passed. At the user's request, task background
processes were stopped and remaining unit/affected verification was waived
before publishing directly to remote dev. This is visual acceptance of the
reported Zeno issue, not a full version-matched rendering parity gate.

## Vanilla skull correction

Current head identities have one shared mapping in `assets::vanilla_skull_type`.
The compiler gives them an Invisible terrain route: their model belongs to the
block-actor renderer. Description chooses the model from the current block name,
even when `SkullType` is absent or stale; the legacy unsplit `minecraft:skull`
continues to use that NBT field. The old compiler recognized only the unsplit
name, so current head blocks emitted terrain fallback geometry that could cover
the correctly lit block-actor geometry. This is a separate native-head mismatch;
fixing it alone did not resolve Zeno's custom-block light decoding.

Placed skulls retain their block and sky levels in `BlockEntityLight::Actor`.
Both the head and hat emit outward world normals after mounting and yaw,
without terrain face coefficients. The block-entity solid shader uses the
shared actor lightmap, normal shading, gamma transfer and distance fog.
Environment changes update the shared lightmap even when cached geometry is
reused. Other block-entity models retain their scalar lighting path.

The previous scalar `max(block_curve, sky_curve * daylight)` bypassed colored
light, ambient adjustment, brightness and effects, reaching exactly zero in
dark cells. Fixed local terrain coefficients also failed to follow rotated
heads. The correction derives these behaviors from the native material path;
it adds no brightness multiplier or minimum-light override.

Focused mesh regressions check floor/wall normals, both layers, retained light
levels, cache invalidation and isolation from later cracks. Metal readback
tests execute production mesh and shader entry points against numeric
day/night/torch/dark/brightness witnesses, rotations, alpha cutoff and the
retained scalar path. Registry-wide compiler regressions reject terrain models
for current heads, and description regressions cover missing/stale NBT.
Native hat geometry, other block-entity materials and full version-matched
visual parity remain separate gates.
