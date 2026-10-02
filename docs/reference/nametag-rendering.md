# Native name-tag rendering source record




The original behavior was a screen-space/HUD approximation: quantized font scale clamped
to the HUD's minimum/maximum, a one-line plate behind multiline text, a separate Java-style
score renderer, and collision-ray whole-tag occlusion. The merged upstream path retains the
shared UI font-layout cache and glyph-line atlas, publishing compact font-pixel records to
the environmental-text GPU renderer. The shader constructs homogeneous world quads for
perspective-correct glyphs and near-plane clipping, preserving the verified native cubic
billboard, base anchor and multiline lift rather than the upstream exact-angle/text-shift
approximation. World tags render before first-person hands and HUD. Plates do not write depth;
glyphs do. Sneak tags additionally test world depth, while ordinary tags use Always.

## Verified geometry and constants


| Meaning | Native witness | Value |
| --- | --- | --- |
| Ordinary black background | `DAT_150102660` | RGBA `(0, 0, 0, 0.25)` |
| Head clearance above adjusted AABB height | `DAT_15005ea04` | `0.7` blocks |
| Font world scale | `DAT_1500de2e0 * DAT_15006c0b0` | `1.6 * (1/60)` blocks/design pixel |
| Line pitch | `DAT_14ffab650` | `10` design pixels |
| Background top and horizontal padding | `DAT_14fee6588`, `DAT_14fea4060` | `-1`, `1` design pixels |
| Extra world lift per additional line | `DAT_14ff1b14c` | `0.125` blocks |
| Sneaking foreground alpha | `DAT_14ff1b14c` | `0.125` |
| Billboard zero-horizontal fallback | `DAT_15005b1dc` | `0.0001` |
| Cubic acos coefficients | `DAT_1500f2cec`, `DAT_1500f2ce8` | `0.87266463`, `-0.69813168` |
| Below-name score distance squared | `DAT_14ffa2648` | `<100` blocks squared |

For each nonempty LF-separated line the native font width is halved with integer truncation.
Every line starts at its own negative half-width. The plate uses the maximum half-width,
plus one design pixel on each side, and spans `-1 .. 10 * line_count - 1` vertically.
The billboard rotation uses the original eye-minus-base-anchor direction, while its position
is lifted by `0.125 * (line_count - 1)`. Local X and Y are both negated by the world scale.
There is no text shadow. The accepted open-font deviation remains: our font atlas is converted
using the shared `ui::FONT_DESIGN_PIXEL_TEXELS` instead of duplicating its texel density.

The current extraction caller appends actor data 84 as a new line only inside the score
distance gate. Our retained below-name objective supplies `score + space + display name`
when the server did not supply actor-data score text; empty synced text remains authoritative.
Render-distance metadata is key 140 on this target (the old tracking table listed a different
version's key). An empty synced player name hides its tag; a missing tag falls back to its spawn
username, without changing its scoreboard name authority.

## Material and shader witnesses

The installed PlayCover app and the IPA named `Minecraft-1.26.50-for-iOS-mcpelife.ipa`
contain the definitions in `data/resource_packs/vanilla/materials/ui3D.material`, lines
224–345, and readable Metal shaders in `data/renderer/materials/Nametag.material.bin`
and `UIText.material.bin`. Their **internal version is 1.26.51.01**, not the filename's
1.26.50; they corroborate the matched C++ but are not a version-matched shader-pack witness.
SHA-256 values: `ui3D.material`
`2ca6efaa1e93d650c2476025cb4d6043a70a14516ae542ffcb4218dc8abeeaec`,
`Nametag.material.bin`
`cf0f7d60b85c42324fa2955c50a599405bdbaf238c0d800b9eeeefb17ff98d49`,
`UIText.material.bin`
`b1f6dc57ea38b7ececf55ec1209b5521e005e192bf05fe0b08ee6ea01e85eb2b`.

`name_tag` blends with DisableDepthWrite and Always; `name_tag_depth_tested` inherits
the plate and changes to LessEqual. `name_tag_text` blends with Always and does not
disable depth writing. The depth-tested text is named **`name_text_depth_tested`**;
it inherits `sign_text`, enables ALPHA_TEST, and uses LessEqual. The shader discards
sampled glyph alpha below 0.5 **before** multiplying text opacity, so sneak text at
0.125 opacity must survive. Text and background use different compiled shaders.


## Acceptance and incomplete branches

Focused tests cover world size/rotation, multiline background/centering/lift, sneaking alpha,
metadata overrides, score distance, homogeneous projection, safe-area/DPI independence,
texture-page ordering and render graph layering. This is not a closed visual parity gate.

Native material definitions and compiled shaders have now been located as described above;
the prior speculative collision-ray dimming is removed. A matched 1.26.50 shader-pack witness
and native fixed-state comparison are still needed to close exact through-wall/material parity.
Explicit orientation/backface/education/custom-color paths, profanity-filtered names,
vehicle/riding-adjusted anchors, default missing height/render distance, and the inherited
provisional crosshair selection shapes/range remain incomplete. The presentation budget of
128 nearest tags is an upstream client resource bound, not a vanilla visibility rule. Player-default
visibility, current-position/camera-anchor distance details, the retained-score fallback and
NameplateDepthTested flag 129 (outside the current wire flag carrier) remain incomplete.

The canonical debug build was run in Zeqa on 2026-10-01: macOS 26.3, Apple M3 Pro,
Metal, 1280×720 logical viewport, 2560×1440 physical pixels (DPI scale 2). The endpoint
resolved to `40.223.14.30:19132`, advertised protocol 2193 / game 1.26.50 / WaterdogPE
Proxy, and transferred pre-login to `pvp.inpvp.net`. Rendered frames confirm visible
translucent multiline plates, individually centered colored lines, perspective-scaled
text, and first-person/HUD coverage. Native window capture and client F2 frames are
retained privately under `.local/screenshots/`; no game artwork is added to git.
The exact tested executable hash and scenarios are in the ignored acceptance record.

Movement and different camera pitches rendered successfully, but the captures do not
form a controlled same-actor near/far comparison: capture focus changes and concurrent
input altered the view, and one approach opened an NPC form. No purchase/form action
was submitted. Controlled anchor drift, sneaking/through-wall depth, near-plane clipping,
and version-matched vanilla fixed-state frame comparisons remain pending. This is a
live rendering smoke pass, not a closed visual parity gate or performance acceptance.
