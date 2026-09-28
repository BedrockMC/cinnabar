# Vanilla-parity gap tracker


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
Held/offhand items are decoded+stored but never drawn; remote armor is decoded then
dropped (`sequencing.rs:688`); the first-person near-camera rig pass exists but is fed
`None`. Vanilla binds item/armor geometry to the biped bones (`rightItem`/`leftItem`
exist; non-arm bones zero-scaled for first person) via attachables. 6 tranches: **T0 asset
ingestion — landed** (entity compiler now collects `attachables/` + `textures/models/armor/`;
per-item `EquipmentBinding` table — geometry/texture/material/render-controller, armor `.player`
variant preferred so its geometry resolves in-catalog — emitted to a new hash-pinned `.mcbeeqp`
carrier; only the trident's `wield_first_person`/`wield_third_person` are literal and populate
`ItemVisualDefinition`, everything Molang/query-derived is flagged `NeedsMeasurement`; no
fail-closed startup bail until a consumer lands). Next: T1 third-person held item, T2 worn armor
(both layers, tiers, dye), T3 first-person arm + held item (populate the disabled pass — replaces
the removed stopgap), T4 offhand + shield/elytra/pumpkin-head/bow-frames, T5 polish (glint,
trims, PBR).

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
- Weather precipitation: procedural rain/snow sheets, biome/height classification and per-column surface limits landed uncompiled; `weather.png` not yet carried, splash particles and bolt renderer unwired *(measure)*.
- Particles — no system at all; block-break etc. absent (HIGH).
- Daylight: eased celestial angle, day plateau and night transfer landed uncompiled; the shader night floors
  (`lighting.wgsl`, `chunk/gpu/upload/lighting.rs`, cloud) still clamp at 0.2/0.04 and must drop to `NIGHT_SKY_TRANSFER` (HIGH).
- Stars: procedural star field landed uncompiled; twinkle unverified *(measure)*.
- Leaves: Fancy look landed (leaf↔leaf faces kept); live compare pending.
- Block-entity models (chests, beds, shulkers, banners, skulls, conduit, bell) + sign text — absent (MED-HIGH).
- Server resource packs: custom blocks (sequential and hashed ids), item icons, and lang apply at runtime;
  vanilla block/entity retexturing, custom entities, and merged sounds/ui consumption remain unapplied (HIGH).
- Sky now biome-temperature-derived, fog linear and rain-blended; clouds uncalibrated, End sky texture uncarried,
  sun/moon quad size, AO darkening step, water surface alpha *(measure)*.

## HUD (Java target; chat/scoreboard intentionally Java — not gaps)
- Title/subtitle/action bar: left-anchored, unscaled, no fade — should be centered, scaled, alpha-faded (HIGH).
- Screen overlays absent: vignette, portal, underwater, fire, powder-snow, spyglass scope (MED). No red damage flash is correct.
- Boss-bar colors/notches approximate; toasts unboxed; no heart jitter/regen bob; no food shake; effect-blink approximate; offhand handedness; hardcore hearts (LOW).
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
