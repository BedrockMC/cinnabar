# Graphics options expander

The requested full-height chevron row differs from the supplied pack. Its
`ui/settings_sections/general_section.json` declares a 20-pixel
`advanced_graphics_options_button` and plus/minus artwork in
`advanced_graphics_options_button_content` (line 10975). The original screenshot
shows the open/minus state, rather than a failed icon load.

Cinnabar uses that same controller, visibility bindings and option grid, with the
requested taller treatment. Height comes from the installed
`settings_common.action_button` (30 pixels, line 76), and the icons use the pack's
`textures/ui/arrowRight` and `textures/ui/arrowDown`. The label stays vertically
centred. Server UI overlays are applied afterward.

References inspected:


The json-ui test preserves the pack's actual 20-pixel plus/minus baseline. The
client test verifies the requested full-height hit target, controller action and
collapsed/expanded arrows. Offline snapshots render both catalogs through the
real carrier. This adaptation does not close a vanilla visual parity gate.
