//! World-space actor name tags: camera-facing billboards of backing plates and rasterized text,
//! laid out in font pixels as the vanilla name-tag renderer does.
use std::sync::Arc;

/// World size of one font pixel on a tag (vanilla scales the tag by 1.6 / 60).
pub const NAMETAG_BLOCKS_PER_FONT_PIXEL: f32 = 1.6 / 60.0;
/// Text sits this far in front of its plate, toward the camera.
pub const NAMETAG_TEXT_LIFT_BLOCKS: f32 = 0.01;
/// Side of the square RGBA8 text atlas.
pub const NAMETAG_ATLAS_SIDE: u32 = 2048;
/// Most plate and text quads drawn in one frame.
pub const MAX_NAMETAG_RECORDS: usize = 1024;

/// One quad of a tag as the GPU reads it: a plate when `uv[2] < 0`, else a text line.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NametagRecord {
    /// World point the tag's font-pixel origin hangs at.
    pub anchor: [f32; 3],
    /// 1 for text, pushed [`NAMETAG_TEXT_LIFT_BLOCKS`] toward the camera; 0 for a plate.
    pub text: u32,
    /// `[x0, y0, x1, y1]` in font pixels; +x is the viewer's right, +y is down.
    pub rect: [f32; 4],
    /// Normalized atlas rect `[u0, v0, u1, v1]`; `u1 < 0` draws the flat colour.
    pub uv: [f32; 4],
    /// Straight-alpha RGBA multiplier.
    pub color: [f32; 4],
}

/// Immutable pixels for one non-overlapping atlas cell.
#[derive(Clone, Debug, PartialEq)]
pub struct NametagAtlasRect {
    /// Atlas x, y, width and height in texels.
    pub cell: [u32; 4],
    pub rgba8: Arc<[u8]>,
}

impl NametagAtlasRect {
    /// Returns changed cells, including updates missed between render extractions.
    pub fn updates<'a>(
        current: &'a [Self],
        previous: &'a [Self],
    ) -> impl Iterator<Item = &'a Self> {
        current
            .iter()
            .enumerate()
            .filter_map(move |(index, rectangle)| {
                let unchanged = previous.get(index).is_some_and(|old| {
                    old.cell == rectangle.cell && Arc::ptr_eq(&old.rgba8, &rectangle.rgba8)
                });
                (!unchanged).then_some(rectangle)
            })
    }
}

/// This frame's tags for the render world. Records before `see_through` draw over everything;
/// the rest are depth tested.
#[derive(
    bevy::prelude::Resource,
    bevy::render::extract_resource::ExtractResource,
    Clone,
    Debug,
    Default,
    PartialEq,
)]
pub struct NametagScene {
    pub records: Vec<NametagRecord>,
    pub see_through: usize,
    /// All live cells, sharing unchanged pixels across publications.
    pub atlas: Arc<[NametagAtlasRect]>,
    pub atlas_revision: u64,
}
