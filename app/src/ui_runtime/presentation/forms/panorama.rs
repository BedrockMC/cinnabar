//! Feeds the launcher's panorama pass: decodes the carrier's six faces once and
//! turns the camera each frame while a launcher screen is up.

use std::{
    f32::consts::{PI, TAU},
    io::Cursor,
    sync::Arc,
    time::Instant,
};

use assets::RuntimeUiAssets;
use bevy::{
    prelude::{Local, Query, Res, ResMut, With},
    window::{PrimaryWindow, Window},
};
use image::{ImageFormat, ImageReader, Limits};
use render::{MAX_PANORAMA_FACE_SIDE, PanoramaFaces, PanoramaScene, PanoramaView};

use super::super::UiPresentationRuntime;
use crate::menu::{MenuRuntime, MenuScreen};

/// Vertical field of view of the panorama camera.
const VERTICAL_FOV: f32 = 85.0 * PI / 180.0;
/// Seconds per full turn.
const TURN_SECONDS: f32 = 360.0;
/// Constant downward tilt of the camera.
const PITCH: f32 = 0.0;

/// Whether the carrier holds the panorama, so the launcher leaves its backdrop clear.
pub(super) fn carried(assets: &RuntimeUiAssets) -> bool {
    assets.ui_file("textures/ui/panorama_0.png").is_some()
}

/// Uploads the faces on first sight of the carrier and shows the panorama
/// behind launcher screens (never behind the in-game pause or death screens).
pub(crate) fn drive_menu_panorama(
    presentation: Res<UiPresentationRuntime>,
    menu: Option<Res<MenuRuntime>>,
    scene: Option<ResMut<PanoramaScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut state: Local<Option<(Instant, [f32; 4])>>,
) {
    let Some(mut scene) = scene else {
        return;
    };
    let Some(engine) = presentation.form_presentation.engine.as_deref() else {
        scene.show(None);
        return;
    };
    if state.is_none() {
        let assets = engine.assets();
        *state = Some((Instant::now(), overlay_tint(assets)));
        scene.set_faces(decode_faces(assets).map(Arc::new));
    }
    let shown = menu.as_ref().is_some_and(|menu| {
        menu.is_visible() && !matches!(menu.screen(), MenuScreen::Pause | MenuScreen::Death)
    });
    let aspect = windows
        .iter()
        .next()
        .map(|window| window.width() / window.height().max(1.0))
        .unwrap_or(16.0 / 9.0);
    let Some((epoch, tint)) = *state else {
        return;
    };
    let turned = (epoch.elapsed().as_secs_f32() / TURN_SECONDS).fract() * TAU;
    scene.show((shown && scene.has_faces()).then_some(PanoramaView {
        yaw_radians: turned,
        pitch_radians: PITCH,
        vertical_fov_radians: VERTICAL_FOV,
        aspect,
        tint,
    }));
}

/// The six faces at their native, equal size; any missing or odd face drops them all.
fn decode_faces(assets: &RuntimeUiAssets) -> Option<PanoramaFaces> {
    let mut side = None;
    let mut faces = Vec::with_capacity(6);
    for face in 0..6 {
        let bytes = assets.ui_file(&format!("textures/ui/panorama_{face}.png"))?;
        let (width, height, pixels) = decode_png(bytes)?;
        if width != height || side.is_some_and(|side| side != width) {
            return None;
        }
        side = Some(width);
        faces.push(pixels);
    }
    let faces: [Vec<u8>; 6] = faces.try_into().ok()?;
    PanoramaFaces::new(side?, faces)
}

/// The 1x1 overlay's colour as straight-alpha floats; clear when absent.
fn overlay_tint(assets: &RuntimeUiAssets) -> [f32; 4] {
    assets
        .ui_file("textures/ui/panorama_overlay.png")
        .and_then(decode_png)
        .and_then(|(_, _, pixels)| pixels.get(..4).map(|p| p.to_vec()))
        .map_or([0.0; 4], |p| {
            [p[0], p[1], p[2], p[3]].map(|channel| f32::from(channel) / 255.0)
        })
}

fn decode_png(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_PANORAMA_FACE_SIDE);
    limits.max_image_height = Some(MAX_PANORAMA_FACE_SIDE);
    limits.max_alloc = Some(u64::from(MAX_PANORAMA_FACE_SIDE).pow(2) * 4);
    reader.limits(limits);
    let image = reader.decode().ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some((width, height, image.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pngs_decode_to_rgba_with_their_size() {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 40]))
            .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        let (width, height, pixels) = decode_png(&bytes).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(&pixels[..4], &[10, 20, 30, 40]);
        assert!(decode_png(b"not a png").is_none());
    }
}
