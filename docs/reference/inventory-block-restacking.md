# Bare block stack identity and crafting admission


## Native source chain

Paths in this section are relative to the local reconstruction checkout.
Current unmapped paths begin `current/1.26.50.26/src/`; named reference paths
begin `reference/26.30/src/by-owner/i/`.


Native occupied-stack compatibility is not a requirement for zero aux or zero
block identity:

- `matchesItem` compares item definition identity, aux (either `0x7fff` is a
  wildcard), user data, restriction hashes and an additional field at `+0x70`.
  A present left-hand block pointer at `+0x18` must match the right-hand block
  pointer; merely being present is not a rejection. Its charged-item path has
  further checks and is outside this fix.
- `isStackable(other)` requires the same item definition pointer and a stackable
  other stack. It compares aux when the item's variant flag requires it, then
  user data, the restriction hashes and the additional `+0x70` field. This
  function does not itself compare the block pointer at `+0x18`.
- `isStackable()` asks the item for its descriptor-dependent maximum stack size.
  A maximum below two is not stackable. Damaged damageable items have additional
  damage/Unbreakable conditions; a nonzero aux alone is not a universal refusal.
- `hasSameUserData` compares present compound tags structurally. Missing user
  data and an empty compound tag can compare equal. Native support is broader
  than admitting only empty serialized data.


## Recipe identity is a separate check




## Observed failure and scoped correction

The offline loopback vanilla BDS baseline reproduced two local refusals:

- Taking 32 blocks received an Accepted response. Left-clicking to restack
  them then issued no request; this was client admission, not a rejected merge.
- A grid containing eight oak logs showed no crafting output.

Both paths shared the old `plain_stack` guard, which required aux and block
runtime identity to be zero as well as empty user data. Ordinary block stacks
carry nonzero block runtime identity, so they were incorrectly classified as
unsupported for both occupied-stack gestures and recipe matching.

`app/src/ui_runtime/inventory_ledger/registry.rs` now separates these concerns:

- `plain_stack` checks only the two already supported empty extra-data
  encodings (empty bytes or ten zero bytes) and their valid SHA-256 digest.
  Nonzero aux or block runtime identity does not make user data non-plain.
- `occupied_stack_relation` compares source/destination aux and block runtime
  identity for equality, permitting equal nonzero values. It retains negotiated
  identifier/capacity checks and sparse/server authority validation.
- Manual grid matching and auto-craft admission continue to check recipe item
  or tag identity and the ingredient's aux rule independently. The change
  admits bare block ingredients and bare nonzero-aux ingredients only where
  that separate recipe rule accepts them.

The existing no-meaningful-overlay guard on compatible merges remains in place.
Named, enchanted or otherwise nonempty user data remains unsupported for this
merge prediction path, even when native structural comparison would accept it.
The scoped equality checks also do not reproduce native variant-flag exceptions,
wildcard full-stack matching, descriptor-dependent capacity/damage behavior or
charged-item comparison. This is not a claim of complete inventory parity.

## Verification status

Regressions in `app/src/ui_runtime/inventory_ledger/merge_tests/blocks.rs`
cover accepted and still-pending block split/restack, preservation of nonzero
runtime identity, equal nonzero aux, refusal to merge different aux/runtime
identities, and the two supported empty user-data encodings. These are focused
contract tests, not a substitute for live server acceptance. Post-fix live
acceptance and final verification are recorded by the task owner separately.

The Accepted BDS split/restack requests and independent server quantity checks
are recorded in [the correction acceptance record](../reviews/inventory-hud-crafting-fixes.md).
