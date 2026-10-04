# Camera FOV projection

The full-window camera contract was inspected through Lens against the pinned
26.30 client. Cinnabar independently implements that contract; native source
is not copied.

`LevelRendererPlayer::getFov` (`0x1043a8820`) and `getFovWithoutGameplay`
(`0x1043a8bb0`) scale the configured angle by
`min(normalized_viewport.y / normalized_viewport.x, 1)`. The virtual method at
ClientInstance slot `0x630` is `ClientInstance::getNormalizedViewportSize`
(`0x102341790`): each viewport dimension is divided by the corresponding
full-screen dimension from `GuiData::ScreenSizeData`. The values are `(1, 1)`
for a full-window viewport at any display aspect ratio. This scale adjusts
partial viewports, such as split screen; it is not pixel height divided by
pixel width.

`CameraAPI::tryGetFOV` (`0x1007f8220`) converts that angle to radians.
`dragon::rendering::Camera::createPerspective` (`0x10c0c6500`) supplies the
viewport width/height separately to `bx::mtxProjRh` (`0x100344300`). That
projection places `cot(FOV / 2)` on the vertical axis and divides it by aspect
on the horizontal axis. The full-window setting therefore defines the
vertical FOV. A setting of 110 degrees stays 110 degrees vertically, about
137 degrees horizontally at 16:9.

Cinnabar previously treated this setting as a horizontal angle and converted
it through `2 * atan(tan(FOV / 2) / aspect)`. At 16:9, setting 110 consequently
rendered only about 77.55 degrees vertically. The camera now uses the setting
directly as vertical radians and supplies aspect independently. The legacy
`horizontal_fov_degrees` settings field and accessor remain compatible with
existing saved settings; their name does not describe the corrected axis.

Tests cover configured angles, landscape and portrait projection matrices,
the live 110-degree settings handoff, malformed values, startup, and window
resize. This establishes the full-window base projection contract. Dynamic
gameplay FOV magnitudes remain provisional, and split-screen viewport scaling
is not implemented by the current single-window client.
