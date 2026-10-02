# Bare block stack identity and crafting admission

Reference: local `mcsrc-1.26.50`, reconstruction revision
`da728f0ce4d7a5ae0be443b8abe03119858d923e`, current Windows client
`1.26.50.26`, matching PE SHA-256
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.
The named 26.30 reference identifies the functions; current source and the
matching PE descriptor table corroborate the contracts below. Our code is
independently written, not copied decompiled C++.

## Native source chain

Paths in this section are relative to the local reconstruction checkout.
Current unmapped paths begin `current/1.26.50.26/src/`; named reference paths
begin `reference/26.30/src/by-owner/i/`.

| Contract | Named 26.30 RVA and file line | Current RVA and file line |
| --- | --- | --- |
| `ItemStackBase::isStackable()` | `0a6732b0`, `ItemStackBase.cpp:3190` | `0278c9c0`, `__unmapped/02.cpp:1255933` |
| `ItemStackBase::isStackable(other)` | `0a673520`, `ItemStackBase.cpp:3236` | `0278cd10`, `__unmapped/02.cpp:1256083` |
| `ItemStackBase::hasSameUserData` | `0a68ab80`, `ItemStackBase.cpp:7280` | `0278aa00`, `__unmapped/02.cpp:1254508` |
| `ItemStackBase::matchesItem` | `0a68f910`, `ItemStackBase.cpp:10821` | `027917e0`, `__unmapped/02.cpp:1258966` |
| `ItemDescriptor::sameItem(other, checkAux)` | `0a67d2b0`, `ItemDescriptor.cpp:3735` | `027e9260`, `__unmapped/02.cpp:1317615` |
| Concrete item descriptor comparison | Current PE corroboration | `0282f9b0`, `__unmapped/02.cpp:1367227` |
| Stack-to-descriptor construction | Current source corroboration | `02789dc0`, `__unmapped/02.cpp:1253974` |

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

The current debug formatter at RVA `02787f40`,
`__unmapped/02.cpp:1252700`, labels `+0x68` as CanDestroyHash and `+0x48` as
CanPlaceOnHash. Their updater is RVA `02797f10`,
`__unmapped/02.cpp:1263465`. The extra `+0x70` comparison is retained as an
identified field, not assigned an unverified semantic name. For ordinary
occupied stacks, count and server/sparse stack-network IDs are not semantic
item equality keys; they remain necessary for quantities and request authority.

## Recipe identity is a separate check

Current `__recovered/CraftingInputContainerController.cpp:1`, RVA `0368df20`,
calls descriptor comparison with aux checking enabled and identifies the
contract as `sameItemAndAux`. The current recipe-select controller at RVA
`03915fc0`, `__unmapped/03.cpp:1540689`, makes the same check around line
1541779. It separately calls full-stack `matchesItem` when merging items into
an occupied grid slot around line 1541866.

The concrete descriptor comparison at RVA `0282f9b0` compares the item
definition ID. With aux checking enabled, either aux `0x7fff` accepts the
other aux; otherwise aux/state identity must match. Block-backed descriptors
can resolve block states before comparison. The matching PE's descriptor
vtable `150128db0` has this function in its `+0x10` comparison entry, reached
through the descriptor wrapper's `+0x08` entry at RVA `027e32b0`.

Stack-to-descriptor construction at RVA `02789dc0` explicitly handles a
non-null block pointer, including a wildcard block-type descriptor, before
falling back to an item/aux descriptor. None of these ingredient descriptor
comparisons requires zero block identity or universally zero aux, nor do they
compare a full stack's serialized NBT. This does not imply every NBT-bearing
stack is supported by Cinnabar's crafting prediction.

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
