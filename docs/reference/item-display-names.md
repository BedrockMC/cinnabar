# Stack display names

The selected-item HUD label and inventory tooltip resolve the same stack name:
an authoritative stack-response name, then retained NBT `display.Name`, then
the item's localized default. Inventory content and slot packets can carry
custom names without any stack-response correction. Those names must reach
the HUD directly from the presented stack, including during inventory prediction.

## Current native evidence


| Body | Location | Observed behavior |
| --- | --- | --- |
| `027d5c20` | `02.cpp:1304297`, `027d.jsonl:101` | Custom-name presence tests the `display.Name` tag; an empty string still counts. |
| `027d5910` | `02.cpp:1304153`, `027d.jsonl:99` | Reads `Name` and the optional `FilteredName` alternative without stripping format codes. |
| `027945b0` | `02.cpp:1261057`, `0279.jsonl:95` | Custom names override localized defaults; the hover name ends with `§r`. |
| `02794260` | `02.cpp:1260874`, `0279.jsonl:90` | Custom names receive `§o` before the item's formatting and literal name. |
| `01739ba0` | `01.cpp:1283768`, `0173.jsonl:137` | The selected-item popup uses the same hover-name producer at line 1283946. |
| `01738f10` | `01.cpp:1283271`, `0173.jsonl:132` | The popup refreshes when selected slot/source changes, even between identical item kinds (lines 1283490–1283525). |
| `056d6870` | `05.cpp:1163740`, `056d.jsonl:106` | HUD item-text creation explicitly disables localization at line 1164013. |

The matching client executable SHA-256 is
`7d6cf9b2e4b01fce5d6283cc3deb65b877995a8fd1e146f967d8ac743369d628`.
Its Item vtable at image VA `1501217c0` resolves slots `+300`, `+2f8`, and
`+2e8` to the custom-name presence, custom-name reader, and default-name reader
(`027d3f30`) respectively. Image VAs `15006b5d8` and `15006bf90` contain the
`§o` and `§r` literals. These addresses are research provenance, not runtime constants.

## Cinnabar correction

HUD capture previously consulted only stack-response name overlays before
falling back to the localized identifier. The tooltip already read retained
NBT. Both paths now share the name-line resolver, preserving custom text and
its format codes, including explicit resets that override native italics or
component colors. A present empty name stays empty. Unreadable display data
falls back without ending the session. Item-text factory creation also disables
localization, matching the native controller's handling of already-resolved names.
Filtered-name selection remains outside
this correction; it retains the existing unfiltered presentation policy.

The selected-item timer also includes the hotbar slot. It no longer suppresses
the popup when two differently named swords have identical item IDs and metadata.
This does not introduce an inferred same-slot rename timer rule.

Synthetic regressions exercise inventory content through the ledger and real
HUD capture, compare HUD and tooltip names, check slot-change retriggering and
stable-frame timing, and cover empty names, localization, Unicode, unreadable
display data, response precedence and server-authored formatting.

## Validation

On 2026-10-04, the rebuilt macOS client ran on Zeno, whose status response
matched the pinned client game and protocol versions. The user exercised the
custom-name behavior in that client and confirmed that the fix worked.

The three focused selected-item-name regressions passed. Affected-crate
formatting, the architecture gate and compilation also passed. The wider
local test run was stopped at the user's request; Clippy was skipped. Full
local verification is therefore incomplete, and the PR still requires green CI.
