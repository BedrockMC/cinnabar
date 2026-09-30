//! Scene description for the menu panorama cube; the GPU side lives in `panorama_render`.

use std::sync::Arc;

use bevy::{prelude::Resource, render::extract_resource::ExtractResource};

/// Largest accepted face side.
pub const MAX_PANORAMA_FACE_SIDE: u32 = 2048;

/// The panorama shader, for hosts that draw it outside the Bevy render graph.
pub const PANORAMA_WGSL: &str = include_str!("panorama.wgsl");

/// The six square sRGB RGBA8 cube faces, in pack order (`panorama_0`..`panorama_5`).
#[derive(Debug, PartialEq, Eq)]
pub struct PanoramaFaces {
    side: u32,
    pixels: Vec<u8>,
}

impl PanoramaFaces {
    /// Rejects faces that are not all exactly `side` x `side` RGBA8.
    pub fn new(side: u32, faces: [Vec<u8>; 6]) -> Option<Self> {
        let bytes = side as usize * side as usize * 4;
        if side == 0
            || side > MAX_PANORAMA_FACE_SIDE
            || faces.iter().any(|face| face.len() != bytes)
        {
            return None;
        }
        Some(Self {
            side,
            pixels: faces.concat(),
        })
    }

    #[must_use]
    pub const fn side(&self) -> u32 {
        self.side
    }

    /// Face pixels, one face after another.
    #[must_use]
    pub fn layer_major(&self) -> &[u8] {
        &self.pixels
    }
}

/// Where the panorama camera looks this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanoramaView {
    pub yaw_radians: f32,
    pub pitch_radians: f32,
    pub vertical_fov_radians: f32,
    /// Viewport width over height.
    pub aspect: f32,
    /// Overlay tint composited over the faces (straight alpha).
    pub tint: [f32; 4],
}

impl PanoramaView {
    /// The `Panorama` uniform of `panorama.wgsl`: yaw, pitch, tan(half fov), aspect, then tint.
    #[must_use]
    pub fn shader_uniform(&self) -> [f32; 8] {
        let [r, g, b, a] = self.tint;
        [
            self.yaw_radians,
            self.pitch_radians,
            (self.vertical_fov_radians * 0.5).tan(),
            self.aspect,
            r,
            g,
            b,
            a,
        ]
    }
}

/// The panorama drawn behind the launcher; `view` is `None` while it is hidden.
#[derive(Clone, Debug, Default, Resource, ExtractResource)]
pub struct PanoramaScene {
    pub(crate) faces: Option<Arc<PanoramaFaces>>,
    pub(crate) faces_revision: u64,
    pub(crate) view: Option<PanoramaView>,
}

impl PanoramaScene {
    pub fn set_faces(&mut self, faces: Option<Arc<PanoramaFaces>>) {
        self.faces = faces;
        self.faces_revision = self.faces_revision.wrapping_add(1);
    }

    /// Shows the panorama from `view`, or hides it; non-finite views hide it.
    pub fn show(&mut self, view: Option<PanoramaView>) {
        self.view = view.filter(|view| {
            [
                view.yaw_radians,
                view.pitch_radians,
                view.vertical_fov_radians,
                view.aspect,
            ]
            .iter()
            .chain(&view.tint)
            .all(|value| value.is_finite())
                && view.aspect > 0.0
                && view.vertical_fov_radians > 0.0
        });
    }

    #[must_use]
    pub const fn has_faces(&self) -> bool {
        self.faces.is_some()
    }
}

/// Render run condition: world passes queue nothing while the launcher panorama is shown.
pub(crate) fn world_passes_enabled(scene: Option<bevy::prelude::Res<PanoramaScene>>) -> bool {
    scene.is_none_or(|scene| scene.view.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(aspect: f32) -> PanoramaView {
        PanoramaView {
            yaw_radians: 0.0,
            pitch_radians: 0.0,
            vertical_fov_radians: 1.0,
            aspect,
            tint: [0.0; 4],
        }
    }

    #[test]
    fn faces_must_be_six_equal_squares() {
        let face = || vec![0; 4 * 4 * 4];
        assert!(PanoramaFaces::new(4, std::array::from_fn(|_| face())).is_some());
        let mut faces: [Vec<u8>; 6] = std::array::from_fn(|_| face());
        faces[5].pop();
        assert!(PanoramaFaces::new(4, faces).is_none());
    }

    #[test]
    fn degenerate_views_hide_the_panorama() {
        let mut scene = PanoramaScene::default();
        scene.show(Some(view(1.5)));
        assert!(scene.view.is_some());
        scene.show(Some(view(f32::NAN)));
        assert!(scene.view.is_none());
        scene.show(Some(view(0.0)));
        assert!(scene.view.is_none());
    }
}
