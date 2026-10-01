//! Local-only visual check: rasterizes a presentation frame's UI draw input on
//! the CPU (nearest sampling, alpha blending, scissors) into a PNG, so form
//! layouts can be inspected without a window. Written only when
//! `CINNABAR_FORM_SNAPSHOT_DIR` names a directory; images are never committed.

use std::path::Path;

use image::{Rgba, RgbaImage};
use render::{UI_BLEND_INVERT, UiRenderInput, UiRenderVertex};

const SNAPSHOT_ENV: &str = "CINNABAR_FORM_SNAPSHOT_DIR";

/// The frame composited over a mid-grey backdrop.
pub(crate) fn rasterize(input: &UiRenderInput) -> RgbaImage {
    let [width, height] = input.viewport_size;
    let mut image = RgbaImage::from_pixel(width, height, Rgba([70, 90, 110, 255]));
    let pages = input.textures.pages();
    for batch in input.batches.iter() {
        let Some(page) = pages.get(batch.texture_page as usize) else {
            continue;
        };
        let [page_width, page_height] = page.dimensions();
        let pixels = page.pixels();
        let scissor = batch.scissor;
        let indices = &input.indices
            [batch.first_index as usize..(batch.first_index + batch.index_count) as usize];
        for triangle in indices.chunks_exact(3) {
            let corners: [UiRenderVertex; 3] =
                std::array::from_fn(|corner| input.vertices[triangle[corner] as usize]);
            fill(
                &mut image,
                corners,
                |[u, v], color, x, y| {
                    if x < scissor.x
                        || y < scissor.y
                        || x >= scissor.x + scissor.width
                        || y >= scissor.y + scissor.height
                    {
                        return None;
                    }
                    let (u, v) = (
                        (u.floor() as u32).min(page_width - 1),
                        (v.floor() as u32).min(page_height - 1),
                    );
                    let at = ((v * page_width + u) * 4) as usize;
                    let mut texel: [u8; 4] = pixels[at..at + 4].try_into().unwrap();
                    if u32::from(ui::UI_STYLE_GRAYSCALE) & corners[0].style_flags != 0 {
                        let luma = (0.299 * f32::from(texel[0])
                            + 0.587 * f32::from(texel[1])
                            + 0.114 * f32::from(texel[2]))
                        .round() as u8;
                        texel = [luma, luma, luma, texel[3]];
                    }
                    Some(std::array::from_fn(|channel| {
                        (u16::from(texel[channel]) * u16::from(color[channel]) / 255) as u8
                    }))
                },
                batch.blend_mode == UI_BLEND_INVERT,
            );
        }
    }
    image
}

/// Fill one triangle, sampling `shade(uv, color, x, y)` at each covered pixel
/// centre and blending the result over the image.
fn fill(
    image: &mut RgbaImage,
    corners: [UiRenderVertex; 3],
    shade: impl Fn([f32; 2], [u8; 4], u32, u32) -> Option<[u8; 4]>,
    invert: bool,
) {
    let [a, b, c] = corners.map(|corner| corner.position);
    let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if area.abs() < f32::EPSILON {
        return;
    }
    let min_x = a[0].min(b[0]).min(c[0]).floor().max(0.0) as u32;
    let min_y = a[1].min(b[1]).min(c[1]).floor().max(0.0) as u32;
    let max_x = (a[0].max(b[0]).max(c[0]).ceil() as u32).min(image.width());
    let max_y = (a[1].max(b[1]).max(c[1]).ceil() as u32).min(image.height());
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let weight = |from: [f32; 2], to: [f32; 2]| {
                ((to[0] - from[0]) * (p[1] - from[1]) - (to[1] - from[1]) * (p[0] - from[0])) / area
            };
            let (wa, wb, wc) = (weight(b, c), weight(c, a), weight(a, b));
            if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                continue;
            }
            let uv = std::array::from_fn(|axis| {
                wa * f32::from(corners[0].uv[axis])
                    + wb * f32::from(corners[1].uv[axis])
                    + wc * f32::from(corners[2].uv[axis])
            });
            let Some(source) = shade(uv, corners[0].color, x, y) else {
                continue;
            };
            let target = image.get_pixel_mut(x, y);
            let alpha = f32::from(source[3]) / 255.0;
            for channel in 0..3 {
                let over = if invert {
                    255 - target[channel]
                } else {
                    source[channel]
                };
                target[channel] = (f32::from(over) * alpha
                    + f32::from(target[channel]) * (1.0 - alpha))
                    .round() as u8;
            }
        }
    }
}

/// Write `input` as `<dir>/<name>.png` when the snapshot directory is set.
pub(crate) fn write(input: &UiRenderInput, name: &str) {
    let Ok(dir) = std::env::var(SNAPSHOT_ENV) else {
        return;
    };
    let path = Path::new(&dir).join(format!("{name}.png"));
    rasterize(input).save(&path).unwrap();
    eprintln!("snapshot: {}", path.display());
}
