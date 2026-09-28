# Vanilla-parity gap tracker


## Fixed
- Block breaking / interaction: `verified_selection` refused an `Unknown` selected
  slot, blocking all mining before inventory arrived. Now treats it as an empty hand.

## In progress (branches)
- `task/local-player-render` — local third-person body + first-person hand+item, remove non-vanilla corner overlay.
- `task/resource-packs` — server pack download/decrypt/apply (review fixes).
- `task/enhanced-shaders` — opt-in Enhanced render mode (non-parity, off by default).

## Local player rendering
Third-person body (S1) merged: local player routed through the shared animated rig.
Known gap: a one-tick cosmetic body smear on same-dimension teleport/large snap
(`teleported` is hardcoded false; physics exposes no snap signal yet). First-person
hand (M2) not built (near-camera GPU pass).

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
- Sky gradient hand-tuned vs biome-temperature-derived; clouds uncalibrated *(measure)*;
  fog uses smoothstep vs linear; AO darkening step, sun/moon size, water surface alpha *(measure)*.

## HUD (Java target; chat/scoreboard intentionally Java — not gaps)
- Title/subtitle/action bar: left-anchored, unscaled, no fade — should be centered, scaled, alpha-faded (HIGH).
- Screen overlays absent: vignette, portal, underwater, fire, powder-snow, spyglass scope (MED). No red damage flash is correct.
- Boss-bar colors/notches approximate; toasts unboxed; no heart jitter/regen bob; no food shake; effect-blink approximate; offhand handedness; hardcore hearts (LOW).
- Faithful already: hotbar, hearts/armor/absorption, hunger, air, XP, crosshair.

## Camera / view (Bedrock target)
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
