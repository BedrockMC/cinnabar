# Crafting and cursor return on screen close

Reference: local `mcsrc-1.26.50` reconstruction revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current client `1.26.50.26`.
Our implementation is independently written from the following contracts.

## Native contracts

- Current `__unmapped/03.cpp`, `ContainerManagerController::closeContainers`
  `038f36b0` and `_closeContainers` `038f37b0` (around line 1522669): visit
  each return-on-close controller and each of its slots, invoking
  `_returnToPlayerOrDrop` `038e9c40`.
- Named 26.30 `ContainerManagerController.cpp`: `_closeContainers` `09fc95f0`,
  `_returnToPlayerOrDrop` `09fc11b0`, `_autoPlaceOrDrop` `09fc0f50`: auto-place
  into the player's inventory, then Drop what cannot fit. Ingredients are real
  inventory, not a disposable UI preview or an assumed server-side auto-return.
- Named `ContainerFactory::createController` `0a27f760`: cursor and crafting
  input controllers are marked return-on-close; the combined player inventory
  is not. Both carried items and crafting inputs therefore need cleanup.

## Correction

Personal and workbench closes stage cleanup against the same sparse inventory
view used by ordinary transfers. Each input/cursor stack fills compatible partial
player stacks, then empty cells; any overflow becomes a Drop action. The plan is
atomic locally: invalid identities, unavailable authority or queue pressure do
not publish only half a cleanup. Requests retain their prior sparse dependencies.

Close transport waits for those requests and their predecessors to settle and
for the confirmed grid/cursor to be empty. Empty sparse cells with unanswered
requests still require settlement; cancelling them can resurrect an ingredient
in backing inventory. A rejected or incomplete return cancels the pending Close
and restores the retained open surface so the item can be recovered. A close
notification never silently erases an unexpectedly occupied crafting cell.

This acknowledgement-before-Close ordering is Cinnabar's bounded admission
policy, not a claim of identical native tick/flush timing. Arbitrary container
return flags, structural NBT merging, native collection ordering and every
server-initiated-close recovery path remain outside this scoped change.

Regressions cover personal/workbench return, partial-stack plus empty-slot
distribution, full-inventory overflow, cursor overlays, dependent unsent/admitted
requests, invalid identities, rejection recovery and reopen. BDS acceptance is
recorded in [the correction acceptance record](../reviews/inventory-hud-crafting-fixes.md),
including Accepted input/cursor returns, inspected reopened screens and
independent server counts.
