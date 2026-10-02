# Desktop video settings evidence

Scope: the desktop GUI scale modifier and fullscreen setting. The semantic
reference is the configured Lens Bedrock 26.30 binary corpus. The current client
and codec target are defined by [bedrock-target.json](../../assets/bedrock-target.json);
the pinned resource pack is defined by
[vanilla-source.json](../../assets/vanilla-source.json). The desktop behavior is
transferred to that target; this is not a claim of complete version-matched or
cross-platform parity.

## GUI scale

The reviewed Lens functions are:

| Function | Analysis address | Evidence |
| --- | --- | --- |
| `GuiData::calculateOptimalGuiScaleIndex` | `0x102141e90` | Desktop viewport thresholds and zero-based scale index. |
| `GuiData::calculateMaxGuiScaleIndex` | `0x102141c60` | The desktop maximum uses the same thresholds. |
| `GuiScaleCalculator::calculateCurrentOptimalGUIScaleIndex` | `0x102143980` | Bounds the optimal index against the current maximum. |
| `GeneralSettingsScreenController::_getGUIScaleValues` | `0x102d051c0` | Produces viewport-dependent signed modifiers. |
| `GeneralSettingsScreenController::setGuiScaleOption` | `0x102d07ed0` | Applies a modifier relative to the optimal index. |
| `GuiData::calculateGuiScale` | `0x102142710` | Clamps the modifier before mapping the final index to a scale. |
| `GuiData::getClampedGuiScaleOffset` | `0x102142460` | Reports the effective modifier after viewport clamping. |


For an ordinary desktop viewport without safe-zone adjustments, the optimal and
maximum physical scale are `K = clamp(min(width / 376, height / 250), 1, 8)`
using integer division. Settings offer physical scales from `ceil(K / 2)` through
`K`, represented as signed modifiers from `ceil(K / 2) - K` through zero. The
selected modifier is retained across resizing; its applied value is clamped to
the current range. A fixed command-line scale remains a capture override until
the player selects a modifier in settings.

The pinned pack's `ui/settings_sections/general_section.json` defines
`gui_scale_slider@settings_common.option_slider` with control name `gui_scale`.
Its bindings are `#gui_scale`, `#gui_scale_steps`, `#gui_scale_slider_label`,
`#gui_scale_visible`, and `#gui_scale_enabled`. The selected slider position is
the index within the current modifier choices.

## Fullscreen


The same pinned JSON file defines
`fullscreen_toggle@settings_common.option_toggle`, control name `full_screen`,
state binding `#full_screen`, enabled binding `#full_screen_enabled`, and
visibility variable `$show_fullscreen_toggle`.

The Linux host adapter enters Bevy borderless fullscreen on the current monitor
and returns to `Windowed`. F11 handles non-repeated presses for the focused
primary window and remains available when menu input consumes gameplay keys.
Writing back the applied fullscreen state does not publish an unrelated complete
settings replacement, preserving camera and automatic VSync policies.

## Settings spacing

The pinned pack's settings selector and video section add 25 virtual-pixel
spacers when `$settings_spatial_pattern_fix_enabled` is true. Consecutive
spacers survive between video options hidden by their own bindings, producing
the large empty region before Brightness. The desktop context now selects the
pack's compact branch, preserving its normal control sizes and section insets.


## Validation scope

Focused tests cover scale geometry and pointer conversion, viewport-dependent
choices, fullscreen settings/hotkey synchronization, repeat and focus handling,
preservation of unrelated settings, durable preference restoration, and dragging
through native scrolling and scale-induced relayout. Derived display scales use
the text cache's existing fixed-point precision instead of user preference bounds:
physical GUI scale 1 at DPI 2 requires font scale 0.25. On Fedora, all 32 focused
application tests and 26 UI geometry, action, and text tests passed with the
prepared JSON-UI carrier available. They include native-menu control geometry,
font quads, HUD relayout, and pointer alignment at DPI 2. Formatting and the
architecture gate also passed.
The installed release was built with this patch against repository revision
`3c2c142754bc91b32650dd1262198663b3ecd04a`, before integration with the latest
`main`. Its SHA-256 is
`1d9ad8c00bb89324b8a0ae5f5a0f7fe3525b325613eff1a7095188e4f4ee8b22`.
That build passed a live Fedora GNOME/Xwayland pass on 2026-10-01 at
DPI 2, with 1280x720 and 1920x1080 windows and 3840x2560 fullscreen. Checks cover
checkbox/F11 synchronization, held-key repeats, restored window geometry, native
GUI scales 1 through 4, continuous dragging through relayout, pointer alignment,
viewport clamping, and preference restoration across app restarts. Fresh frames
were inspected for legibility, geometry, clipping, layering, scaling, and colors.
The final minimum GUI step renders half the text and control height of the next
step at the same 1280x720 viewport. The installation's binary hash, full scenario
record, and untracked screenshots are retained in
`~/.local/share/cinnabar/logs/video-settings-qa/qa-report.json`.

Language-specific minimum-scale dialogs, safe-zone variants, touch/console rules,
and full native parity are outside this desktop wiring change.
