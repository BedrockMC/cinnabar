# OreUI screens

26.30 draws some screens with OreUI (a web UI bundle, `data/gui/dist/hbui`) instead of JSON-UI.
Cinnabar matches the screens 26.30 uses **by default** and keeps JSON-UI where OreUI is opt-in
or flag-gated. OreUI is drawn entirely in our own code: colours, borders, spacing, type sizes,
states and icons are re-created from reference facts. Nothing from a Minecraft install (images,
fonts, CSS, JS) is loaded, packed, bundled or committed; the open font stands in for the
Minecraft fonts (accepted deviation).

## Which screens are OreUI by default (26.30)

The client picks a tech stack per screen (`ScreenTechStackSelector::getTechStackForScreen`): a
non-zero dev override wins (1 OreUI, 2 JSON-UI), then a preference option, then the screen's
`isSelected() && isSupported()`. Treatment toggles are true only when the service's treatment list
names them, so they default off. Evidence: the 26.30 reconstruction
(`ScreenTechStackSelectorInitializer`, `TreatmentFlightingToggles`, `DisconnectionRequestHandler`,
`OreUIGameplayUtils`) and the local install's `routes.json`.

| Screen | Route | Default | Cinnabar |
| --- | --- | --- | --- |
| Main menu | `/main-menu` | JSON-UI (dev override only) | `start_screen.json` |
| Play | `/play/:tab` | **OreUI** (selected and supported) | OreUI, drawn natively |
| Create / edit world, templates | `/create-new-world`, `/edit-world`, `/start-from-template` | **OreUI** | OreUI where the local-worlds flow uses them |
| Death | `/gameplay/death` | **OreUI** | OreUI |
| Bed | `/gameplay/bedtime` | **OreUI** | OreUI |
| Settings | `/oreui-settings` | JSON-UI unless the `mc-new-settings-screen` treatment (default off; preference default unrecovered) | `settings_screen.json` |
| Disconnected | `/disconnected` | JSON-UI unless treatment toggle 0x42 (default off) | `disconnect_screen.json` |
| Send invites | invite screen | JSON-UI (dev override only) | not built |
| Inventory, trade, containers | `/gameplay/inventory` ... | JSON-UI (dev option only) | Java-styled path (owner exception) |
| Profile, inbox, message, friends drawer, add friend, screenshots, storage, report | various | **OreUI-only** (no JSON-UI switch) | OreUI for profile, inbox, friends drawer |
| Achievements | `/achievements` | OreUI, needs Xbox Live | not built |

The local install used as a visual reference is 1.26.50 (its `Info.plist`), one version newer than
the target; layout facts taken from it need a 26.30 screenshot check.

## Reference screenshots

The PlayCover install can be launched normally from PlayCover to capture reference screenshots
by hand; do not automate its UI, and keep captures out of git (see `AGENTS.md`).
