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

## In progress (branches)
- `task/movement-authinput-parity` — anti-cheat "movement cheats" fix: drop the
  PlayerAuthInput suppression band-aid (send every tick), fix inputless depenetration
  drift, fix pos_delta to be the actual displacement. Needs native A/B on an AC server.
- `task/resource-packs` — server pack download/decrypt/apply (review fixes).
- `task/enhanced-shaders` — opt-in Enhanced render mode (non-parity, off by default).

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
- Weather precipitation (rain/snow) — absent (HIGH).
- Particles — no system at all; block-break etc. absent (HIGH).
- Daylight/lightmap curve wrong: `sin·0.8+0.2` vs vanilla `ramp(cos(easedCelestialAngle))`;
  night too bright (0.2/0.04 floors), no day plateau (HIGH).
- Stars at night — absent (MED-HIGH).
- Leaves: leaf↔leaf faces culled (Fast look) → hollow/speckled; want Fancy (MED-HIGH).
- Block-entity models (chests, beds, shulkers, banners, skulls, conduit, bell) + sign text — absent (MED-HIGH).
- Server resource packs not applied to rendering: core downloads/admits them but
  `application=unavailable`, so custom blocks/textures render as magenta missing-texture (HIGH, confirmed live).
- Sky gradient hand-tuned vs biome-temperature-derived; clouds uncalibrated *(measure)*;
  fog uses smoothstep vs linear; AO darkening step, sun/moon size, water surface alpha *(measure)*.

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
- Dynamic FOV modifiers (sprint/speed/slowness/fly/bow; spyglass 0.1) — absent (HIGH).
- Walk view-bob — absent (HIGH). First-person hand bob evaluator exists but is dead code.
- Mouse sensitivity mapping placeholder `0.002` *(measure)* (HIGH).
- Hurt-direction tilt, nausea/portal warp, spyglass scope — absent (MED).
- FOV projection model (linear-by-aspect vs tangent), default/range *(measure)*; pitch clamp 89.9 vs 90;
  analog look framerate-dependent; server camera instructions decoded but unapplied (MED).

## Movement / physics / controls (Bedrock target)
Core physics binary-confirmed correct (gravity/drag/friction/jump/speed). Gaps:
- Live movement still `FreeCamera`; validated physics not yet the production source (known).
- Sprint activation (double-tap / sprint-on-movement) and toggle-sneak/sprint absent (HIGH).
- Scaffolding empty collision + wrong climb (fall-through); honey block behaviors unconsumed (HIGH).
- Step height 0.6 vs ~0.5625 *(measure)*; swimming pose/swim-sprint; creative flight prediction;
  lava strata; depth strider; scroll-notch magnitude; UI key-repeat (MED/LOW).

## Audio
- Not yet audited. Earlier note: no footstep/block-sound lookups by runtime id exist — likely a large gap.
