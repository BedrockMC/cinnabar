# Vanilla-parity gap tracker

Consolidated 2026-09-28 from a read-only audit against the 26.30 client (Lens +
mcsrc-1.26.50 reconstruction + pinned bedrock-samples). Target: version-matched
vanilla Bedrock, except the in-game HUD, which targets Java Edition by owner
decision — chat and scoreboard styling there are intentional and not gaps.
Values only resolvable from decompiled source are marked *(measure)*.

## Fixed
- Block breaking / interaction: `verified_selection` refused an `Unknown` selected
  slot, blocking all mining before inventory arrived. Now treats it as an empty hand.
- Inventory container routing: re-derived from the vanilla InventoryContent/InventorySlot
  handlers — the dynamic container id keys only generic storage, so a present zero routes
  like absent for every fixed surface (player inv/armor/offhand/cursor). Was dropping the
  player inventory on window 0 / id 29 / dyn Some(0).
- Player skin: loads `.local/assets/skin/player.png` (fallback to generated default),
  uploads it in ClientData instead of the white placeholder, and renders on the local
  body + HUD paperdoll via a synthetic local profile. Minor follow-ups: per-tick skin
  compare could `Arc::ptr_eq` short-circuit; `session.rs` at the 1000-line limit;
  square-only skins; arm_size hardcoded "wide" (slim uploads wide).
- First person: stopped drawing the whole third-person body rig at the camera (it
  occluded the view). Draws no near-camera rig until an arm-only model exists.

## Landed, pending native confirmation
- Movement / anti-cheat "movement cheats": deleted the PlayerAuthInput suppression
  subsystem (send every tick); depenetration resolves per-axis with horizontal clamped
  to intended velocity so an embedded start injects no inputless drift; `pos_delta` is
  the per-tick displacement. Independently reviewed (APPROVE, FIX1 proven red→green).
  Provisional pending native measurement: the retained **vertical** MTV push-out is the
  sole recovery envelope, plus the anchor-probe budgets, the 32-tick history window, and
  the 16-block teleport-snap bound. Owner gate: AC server stops kicking + normal server
  feels unchanged.
- Camera third-person boom: lenient camera collision query skips unknown/unloaded cells
  instead of collapsing to radius 0. Owner: confirm ~4-block boom on servers with custom
  blocks while real walls still shorten it.
- Chat `§k/§l/§o` render (CPU draw-list, not shader — atlas page is per-batch) + translation
  killfeeds resolve. Owner: confirm bold weight / italic shear / obfuscation cadence.

## In progress (branches)
- `task/resource-packs` — server pack download/decrypt/apply (review fixes).
- `task/enhanced-shaders` — opt-in Enhanced render mode (non-parity, off by default).

## JSON-UI engine (clean-room, Bedrock target; Java HUD stays an override)
Owner decision: a faithful 1:1 Bedrock JSON-UI interpreter drives forms, container
screens and menus from the vanilla `ui/*.json` + textures (read at runtime from
`.local/`, never committed); the Java-styled gameplay HUD (`hud_screen` family) stays
on the existing path, never routed through the engine. 8 tranches: **T1 parser+resolver
and T2 length-expr + two-pass layout + nine-slice emit — landed** (`crates/json-ui`,
fixture- and golden-tested against real templates/sidecars; still no app wiring). Next:
T3 bindings + ActionForm/ModalForm (and `grid`/`scroll_view`, laid out as plain panels
for now), T4 input+response, T5 CustomForm controls, T6 chest container, T7 remaining
containers+menus, T8 server-pack overrides. Nine-slice emits as ≤9 self-contained sprite
quads carrying texture path + normalized UV (atlas binds later); no new render primitive.
Layout is deterministic within the virtual root; needs native confirmation: the
physical→virtual UI scale factor (parameterized, not guessed), and three inferred
semantics — omitted `size` = 100% fill while the `default` keyword = natural content
size (image base_size / label text extent); `anchor_to` = parent point and
`anchor_from` = child point (symmetric vanilla dialogs can't distinguish); and no
sub-pixel rounding in-engine (deferred to draw).

## Equipment / attachable rendering (Bedrock 3D target)
T0 landed (attachable bindings, `.mcbeeqp` carrier). Uncompiled/unmeasured lane work now adds
(all **incomplete**; no vanilla acceptance gate closes on it):
- **Carrier v2** carries decoded attachable textures (armor tiers, elytra, ...) beside bindings.
- **Layers:** extra actor rig instances keyed `(session, dim, runtime, layer)` ride the body's pose
  and transform; per-instance dye tint word; instance arena 512 (bodies still 128).
- **Held item (third person, both hands):** flat sprite items only, extruded one texel deep and
  packed into shared atlas pages, on `rightItem`/`leftItem`. The display placement is a
  *provisional* placement, not the retail transform (needs native measurement). Block items,
  bow/crossbow/trident geometry, spyglass/horn poses: not drawn.
- **Worn armor:** four slots from `MobArmorEquipment` (remote and local), player-variant
  geometry bound to body bones by name, tier textures, leather dye from `customColor` (default
  leather colour and colour-space multiply need measurement). No enchant glint/trim/elytra/
  shield/pumpkin head.
- **First person:** near-camera rig pass fed with arm-only masking per the pack's first-person
  part visibility (arm shows for empty hand/map only) plus a drawable held sprite or block cube
  on the posed `rightItem` bone (item atlas bound to the pass); camera-to-rig offset and item
  placement are provisional. Undrawable items keep the CPU icon viewmodel. Eat/drink/bow-draw
  poses are neutral: `query.main_hand_item_use_duration` now counts using-item flag ticks, but
  `max_duration` has no source (no item-use state; only food durations exist in pack data).
- **Block items:** plain opaque cubes in hand (third and first person) and on the head
  (carved pumpkin); non-cube blocks and mob/player heads are not drawn.
- **Elytra:** wings posed from the carrier's literal `default`/`sneaking`/`sleeping` clips;
  gliding and swimming are Molang-driven and fall back to `default`.
- **Not done:** bow/crossbow pull frames, trident geometry, shield re-parent/blocking pose,
  spyglass/goat-horn poses (attachable-to-hand-bone origin semantics and use state need native
  measurement), enchant glint and armor trims.

## Local player rendering
Third-person body (S1) merged: local player routed through the shared animated rig.
Known gap: a one-tick cosmetic body smear on same-dimension teleport/large snap
(`teleported` is hardcoded false; physics exposes no snap signal yet). First-person
hand (M2) not built (near-camera GPU pass).

First-person hand (near-camera draw node) merged; provisional/native-tunable: the
hand's daylight is pinned to 1.0 (won't darken at night), the camera offset/yaw and
FOV need native tuning, and the held-item model isn't attached yet.

Root cause: the local player is never spawned as an actor, so it never gets the
animated rig remotes use. All three below flow from that.
- Third-person body: static 6-bone diagnostic biped placed at camera yaw (no body/head
  split, no walk/idle/swing). Implement: spawn a client-fed local actor, route through
  the shared rig + motion model.
- First-person hand+item: flat CPU sprite / static `EmptyHandNeutralStaticFallback`; no
  bob/swing/equip/sway/lighting/held item. Implement via first-person render controller +
  `animation.player.first_person.*` (public samples) + `ItemInHandRenderer` transforms (Lens).
- Top-left mini player: no vanilla counterpart → remove if a standalone overlay; keep the
  inventory/menu paperdoll.

## Inventory / containers (Java target)
- Missing screens (force-closed at admission): furnace/blast/smoker, anvil, enchanting,
  grindstone, loom, smithing, cartography, stonecutter, brewing, beacon, hopper,
  dispenser/dropper, **creative**, recipe book. Only chest(0)/workbench(1) admitted.
- Interactions missing/partial: hover highlight (hit already computed, unused), tooltips,
  click-drag distribute, double-click gather, whole-stack shift-click (moves one dest only),
  craft-all / number-key over output, creative pick (backend exists, no caller/screen).
- Polish: empty-slot ghost icons, inventory durability bars, real arrow/enlarged output cell,
  drop the non-vanilla "Crafting" title, use server container titles.
- Panel palette/slot geometry already correct.

## World rendering / atmosphere (Bedrock target)
- Weather precipitation: procedural rain/snow sheets, biome/height classification and per-column surface limits landed uncompiled; optional `make weather-assets` carrier supplies the vanilla rain band and End sky (procedural when absent); biome samples are averaged on a provisional 27-point lattice; additive bolt renderer and flash trigger from `lightning_bolt` actors landed; splash and rain-sound consumers (`RainSplashQueue`, `PrecipitationMix`) are unwired *(measure)*.
- Particles — data-driven engine landed (`crates/render/src/particles`, `make particle-assets`),
  **incomplete, uncompiled and unverified**: block break/crack, LevelEvent and
  SpawnParticleEffect triggers, critical hits. Provisional *(measure)*: break/crack piece
  counts and radii, particle size as half-extent, `particles_alpha` treated as blended
  with a low alpha cutoff, collision-drag model, light mapping, spawn/draw distance caps,
  Molang variable wire layout. Missing: biome tint on break pieces, item-icon particles,
  local mining crack trigger, entity/animation-driven emitters (spawn API exists),
  particle sound routing, `emitter_bound`/travel-distance events (HIGH until measured).
- Daylight: eased celestial angle, day plateau and night transfer landed uncompiled; the shader night floors
  (`lighting.wgsl`, `chunk/gpu/upload/lighting.rs`, cloud) still clamp at 0.2/0.04 and must drop to `NIGHT_SKY_TRANSFER` (HIGH).
- Stars: procedural star field landed uncompiled; twinkle unverified *(measure)*.
- Leaves: Fancy look landed (leaf↔leaf faces kept); live compare pending.
- Block-entity models: chests (single/double, lid cue), beds, shulkers, banners, skulls, bell, enchant/lectern book, beacon
  beam, end portal, sign text, break-crack overlay are drawn from the `.mcbeben` carrier but uncompiled/unmeasured
  and lit only by retained light; conduit, pots, campfire, frames, spawner, dragon/piglin heads, banner/beam
  scroll and hanging-sign extents remain absent or provisional (MED-HIGH).
- Server resource packs: custom blocks (sequential and hashed ids), item icons, and lang apply at runtime;
  vanilla entity retexturing, custom entities, custom-block selection boxes, and audio consumption of merged sounds remain unapplied (HIGH).
- Sky now biome-temperature-derived, fog linear and rain-blended; clouds uncalibrated, End sky from the optional carrier,
  sun/moon quad size, AO darkening step, water surface alpha *(measure)*.

## HUD (Java target; chat/scoreboard intentionally Java — not gaps)
- Title/subtitle/action bar centered, magnified, alpha-faded from SetTitle timings; placement constants need measurement (uncompiled).
- Screen overlays: see the camera section (dedicated overlay pass landed uncompiled; underwater overlay not listed there) (MED). No red damage flash is correct.
- Boss-bar colors/notches approximate; effect-blink approximate; hardcore hearts (needs carrier roles + hardcore flag) (LOW). Heart jitter/regen wave, hunger shake, boxed sliding toasts, distance-scaled player nametags added uncompiled; nametags lack wall occlusion and mob tags. Offhand handedness has no Bedrock source.
- Chat/killfeed: unicode and format-code glyphs not rendering (open font / text renderer
  coverage) — garbled server killfeed text (MED, confirmed live).
- Faithful already: hotbar, hearts/armor/absorption, hunger, air, XP, crosshair.

## Camera / view (Bedrock target)
- Third-person boom collapses onto the player (camera reads as "too close"): the collision
  avoidance fails closed to radius 0 when the sweep errors or hits geometry; boom radius 4.0
  is itself vanilla-correct. Model height is correct — this is distance only (MED, confirmed live).
- Dynamic FOV (sprint/speed/slowness/flying/bow/spyglass 0.1, tick smoothing, FOV-effects scale), walk view-bob,
  hurt tilt, nausea/portal wobble, server shake and `CameraInstruction` set/clear/fade/FOV are implemented
  presentation-only under `app/src/camera/`; every magnitude, curve and sign is provisional *(measure)* (MED).
- Screen overlays (pumpkin blur, spyglass scope, portal, freezing, suffocation, fire, server fade) draw in a dedicated
  pass (`ScreenOverlayRenderPlugin`); pumpkin and spyglass use the vanilla PNGs when found and procedural art
  otherwise; portal, fire and freezing are procedural, suffocation is a flat tint, and the vignette stays with the
  HUD renderer (MED).
- Presets, target, attach and detach are applied; orbit presets ignore collision. Not applied: view/entity offsets,
  spline instructions, blindness/darkness/night-vision consumers (`VisionEffects`), first-person hand consumer of
  `FirstPersonHandMotion` (MED).
- Look sensitivity now follows a provisional slider curve, gamepad look is frame-rate normalized, optional
  cinematic smoothing; pitch clamp 89.9 vs 90 and FOV range/default still *(measure)* (MED).

## Movement / physics / controls (Bedrock target)
Core physics binary-confirmed correct (gravity/drag/friction/jump/speed). Gaps:
- Live movement still `FreeCamera`; validated physics not yet the production source (known).
- Sprint activation (double-tap / sprint-on-movement) and toggle-sneak/sprint absent (HIGH).
- Scaffolding empty collision + wrong climb (fall-through); honey block behaviors unconsumed (HIGH).
- Step height 0.6 vs ~0.5625 *(measure)*; swimming pose/swim-sprint; creative flight prediction;
  lava strata; depth strider; scroll-notch magnitude; UI key-repeat (MED/LOW).

## Audio
- Not yet audited. Earlier note: no footstep/block-sound lookups by runtime id exist — likely a large gap.

## Entity animation query audit (Bedrock target)

Queries referenced by the vanilla pack's entity, controller, animation and render-controller JSON
(use count in parentheses), by how Cinnabar evaluates them. Sources: `actor_animation/query.rs`,
`tick.rs`. Unlisted queries read 0.

| Status | Queries |
| --- | --- |
| Metadata flag word | is_sneaking, is_sprinting, is_swimming, is_gliding, is_crawling, is_baby (293), is_saddled, is_chested, is_powered, is_tamed, is_sitting, is_angry, is_charging, is_casting, is_eating, is_emoting, is_using_item, is_delayed_attacking, blocking, is_dancing, is_standing, is_playing_dead, plus the remaining `is_*` behaviour flags |
| Metadata value | variant (69), mark_variant, skin_id, model_scale, sit_amount, lie_amount, fuse_time, invulnerable_ticks, swelling_dir, swell_amount (normaliser unmeasured), has_target, get_name |
| Client-derived motion | modified_move_speed (269), modified_distance_moved (226), walk_distance, ground_speed, vertical_speed, position_delta, movement_direction, is_moving, is_on_ground, life_time, anim_time, delta_time, time_stamp (tick count, not world clock), body/head/target x/y rotation |
| Links and equipment | is_riding, has_rider, has_player_rider, is_riding_any_entity_of_type, get_equipped_item_name, is_item_equipped, is_item_name_any, is_sleeping |
| Status / attributes | health, is_alive, hurt_time, hurt_direction, death_ticks, is_shield_powered |
| Item use | main_hand_item_use_duration (ticks the use flag has been set, in seconds) |
| Block sample | is_in_water, is_in_lava (block at the actor's feet, app-fed each frame; is_in_water falls back to the swimming flag or airborne fish before the first sample) |
| Smoothed | swim_amount (ramps toward the swimming flag; step unmeasured) |
| Heuristic | is_grazing (eating flag, unmeasured), standing_scale (unsmoothed 0/1) |
| World / item state | sleep_rotation (bed `direction` state under the sleeper, quarter turns; origin unmeasured), item_is_charged (crossbow `chargedItem` NBT kept on the canonical stack), has_cape (skin carries a valid cape image), property (SyncActorProperty names resolved per entity type; enums read as their value name) |
| Armor | armor_texture_slot, armor_color_slot (equipment store; chainmail, turtle, elytra indices unmeasured) |
| Idle (0) | main_hand_item_max_duration, item_remaining_use_duration, has_head_gear, is_spectator, frame_alpha (evaluated at tick boundaries by design), armor_material_slot, equipped_item_any_tag, kinetic_weapon_*, bone_*/get_root_locator_offset, surface_particle_*, panda counters, wing/tail/shake values, is_levitating, is_jumping |

Engine-fed variables: attack_time, gliding_speed_value, is_holding_right/left, is_sneaking,
is_blocking, damage_nearby_mobs, is_first_person, player_x_rotation, bob_animation, swim_amount,
left/right_arm_swim_amount, has_target (per tick); the rest of the seeded set in `evaluation.rs`
(charge_amount, arm offsets) stays at its seed.

Local player: sneak and sprint come from the latest predicted tick, swim is sprint while in
water, and using and blocking are predicted for bow, trident, spears, spyglass, shield and an
uncharged crossbow while Use is held (food and drink wait for the server flag, since the client
cannot tell whether eating is allowed). Glide, crawl and sleep arrive from server metadata; the
movement simulator models none of them, and sleep_rotation samples the bed under the local rig.

Riders are placed at mount position plus a seat offset rotated by the mount's yaw each tick: the
streamed offset (metadata key 56) when present, else the mount type's `minecraft:rideable` seat
from the local behavior pack (chosen by rider count and unique-id order; absent pack means no
defaults); layouts are picked by the mount's saddled, baby, tamed and sheared flags, and the
seat's `rotate_rider_by` (numeric only) and `lock_rider_rotation` turn the rider's body with the
mount and clamp its head. The offset frame, vertical origin, seat ordering and rotation locks need native
verification. Invisible bodies draw as NoDraw after equipment layers are built, so armor and held
items stay.

Open: cape pixels are retained on the decoded skin but no render path draws them yet;
`armor_material_slot` semantics are unmeasured.
