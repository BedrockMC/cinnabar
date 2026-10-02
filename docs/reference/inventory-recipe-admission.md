# Crafting recipe admission

The protocol catalog retains supported crafting shapes independently of their
recipe-discovery metadata. It does not declare those recipes unlocked.

## Source evidence


## Correction and scope

The previous parser admitted only absent discovery requirements or
AlwaysUnlocked with no discovery ingredients. This removed ordinary vanilla
recipes such as oak planks from the catalog. It also required nonnegative
signed block identities, dropping valid high-bit result identities.

Discovery fields are now traversed with the same descriptor/count/work bounds
but no longer remove a supported crafting recipe. Result block identity is
reinterpreted bit-for-bit as `u32`; item counts, metadata, canonical user data,
recipe dimensions and other existing admission checks remain unchanged.

Client-side limited-crafting/unlocked-recipe gating and recipe-book discovery
state remain incomplete. Actual execution is server-authoritative. This change
does not claim every retained recipe is discovered, does not broaden malformed
framing acceptance, and does not remove bounded admission policies.

Regression fixtures cover the discoverable one-cell oak-planks shape in every
personal/table grid position, high-bit result identity preservation, ignored
discovery contexts and retained nested count limits. Live BDS verification is
separate from these protocol regressions.

Accepted personal/workbench crafts, visible previews and server-verified output
counts are recorded in [the correction acceptance record](../reviews/inventory-hud-crafting-fixes.md).
