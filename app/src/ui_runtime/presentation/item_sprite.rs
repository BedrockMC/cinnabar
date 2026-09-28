//! Straight-alpha item icon pixels for world-space item meshes.

use super::{IconRef, UiPresentationRuntime};

/// One item icon cropped from a UI atlas page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemSpritePixels {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba8: Vec<u8>,
}

impl UiPresentationRuntime {
    /// The icon for `identifier`/`metadata` reduced by an integer factor to fit `max_side`.
    pub(crate) fn item_sprite(
        &self,
        identifier: &str,
        metadata: u32,
        max_side: u32,
    ) -> Option<ItemSpritePixels> {
        let icon = self.item_icon(identifier, metadata)?;
        crop_icon(self, icon, max_side)
    }
}

fn crop_icon(
    runtime: &UiPresentationRuntime,
    icon: IconRef,
    max_side: u32,
) -> Option<ItemSpritePixels> {
    let width = u32::from(icon.uv[2].checked_sub(icon.uv[0])?);
    let height = u32::from(icon.uv[3].checked_sub(icon.uv[1])?);
    if width == 0 || height == 0 {
        return None;
    }
    let page = runtime.textures.pages().get(usize::from(icon.page))?;
    let [page_width, page_height] = page.dimensions();
    if u32::from(icon.uv[2]) > page_width || u32::from(icon.uv[3]) > page_height {
        return None;
    }
    let factor = width.max(height).div_ceil(max_side.max(1));
    if !width.is_multiple_of(factor) || !height.is_multiple_of(factor) {
        return None;
    }
    let (out_width, out_height) = (width / factor, height / factor);
    let pixels = page.pixels();
    let mut rgba8 = Vec::with_capacity((out_width * out_height * 4) as usize);
    for y in 0..out_height {
        for x in 0..out_width {
            // Nearest sample from the middle of each reduced block.
            let source_x = u32::from(icon.uv[0]) + x * factor + factor / 2;
            let source_y = u32::from(icon.uv[1]) + y * factor + factor / 2;
            let offset = ((source_y * page_width + source_x) * 4) as usize;
            rgba8.extend_from_slice(pixels.get(offset..offset + 4)?);
        }
    }
    Some(ItemSpritePixels {
        width: out_width,
        height: out_height,
        rgba8,
    })
}
