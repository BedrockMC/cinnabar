# Selected-item HUD label

The selected-item name uses Bedrock's JSON-UI factory and label template. Its
placement is the repository's approved Java-look HUD exception, not a claimed
native Bedrock pixel placement. Server HUD overrides retain their own templates.

## Native references


## Correction

The built-in Java-look pack had placed its hotbar-clearance offset on the
zero-height `item_text_factory`. Factory-created controls deliberately join the
factory's parent after its siblings (`crates/json-ui/src/bind.rs`,
`Binder::literal_children`), so they do not inherit that offset. The selected
name consequently landed on the hotbar.

Only the built-in pack changes: the item-text role now creates
`java_item_name_text@hud.item_name_text_root`, carrying the pack's existing
hotbar-clearance offset. Native centering, binding, sizing and animation remain
inherited. The existing Java-look survival spacing is unchanged. Vanilla
factory semantics and the jukebox role are unchanged.

The JSON-UI regression renders creative and survival item names against the
installed pinned pack, checks centering, verifies clearance above the hotbar and
survival status rows, and computes expected geometry from the authored root
offset and spacer rather than restating those constants. It also retains the
native animation and the Java-look no-background policy. Live framebuffer
verification remains a separate acceptance step.

The inspected survival/creative framebuffer witnesses and integrated checks are
recorded in [the correction acceptance record](../reviews/inventory-hud-crafting-fixes.md).
