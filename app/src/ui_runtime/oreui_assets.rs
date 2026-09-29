//! Optional OreUI icons read at runtime from the user's own Minecraft install
//! (`data/gui/dist/hbui`): its sprite atlases are packed into one UI page. The
//! images are Mojang's and are never copied, bundled or shipped; without an
//! install the OreUI screens draw their own approximate icons.

use std::{
    collections::HashMap,
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

use image::{ImageFormat, ImageReader, Limits};
use serde::Deserialize;

/// Side of the packed OreUI page.
pub(crate) const OREUI_PAGE_SIDE: u32 = 1024;
const MAX_ATLAS_JSON_BYTES: u64 = 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 1024;
const GUTTER: u32 = 1;

/// The packed OreUI page: RGBA8 pixels (premultiplied) and each bundle image's
/// pixel rect `[x0, y0, x1, y1]`, keyed by its bundle path (`assets/<name>.png`).
pub(crate) struct OreUiImages {
    pub(crate) rgba: Vec<u8>,
    pub(crate) sprites: HashMap<String, [u16; 4]>,
}

#[derive(Deserialize)]
struct AtlasFile {
    name: String,
    width: u32,
    height: u32,
    coordinates: HashMap<String, AtlasRect>,
}

#[derive(Deserialize)]
struct AtlasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Candidate bundle directories in the user's own install: `CINNABAR_OREUI_DIR`,
/// then the platform's default install locations. Nothing is ever copied.
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(dir) = std::env::var_os("CINNABAR_OREUI_DIR") {
        paths.push(PathBuf::from(dir));
    }
    const BUNDLE: &str = "data/gui/dist/hbui";
    if cfg!(target_os = "macos")
        && let Some(home) = std::env::var_os("HOME")
    {
        paths.push(
            PathBuf::from(home)
                .join("Library/Containers/io.playcover.PlayCover/Applications/com.mojang.minecraftpe.app")
                .join(BUNDLE),
        );
    }
    if cfg!(windows) {
        // GDK installs under XboxGames on any drive; UWP packages under WindowsApps.
        for drive in ["C", "D", "E"] {
            paths.push(
                PathBuf::from(format!(
                    "{drive}:\\XboxGames\\Minecraft for Windows\\Content"
                ))
                .join(BUNDLE),
            );
        }
        if let Ok(entries) = std::fs::read_dir("C:\\Program Files\\WindowsApps") {
            paths.extend(
                entries
                    .flatten()
                    .filter(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with("Microsoft.MinecraftUWP_")
                    })
                    .map(|entry| entry.path().join(BUNDLE)),
            );
        }
    }
    paths
}

/// The packed OreUI images from the first bundle found, or `None` with a notice.
pub(crate) fn load_optional_oreui_images() -> Option<OreUiImages> {
    let Some(dir) = candidates()
        .into_iter()
        .find(|dir| dir.join("atlas.json").is_file())
    else {
        eprintln!(
            "no local Minecraft install with an OreUI bundle found (CINNABAR_OREUI_DIR can point at its data/gui/dist/hbui); OreUI screens draw their own icons"
        );
        return None;
    };
    match load(&dir) {
        Ok(images) => {
            eprintln!(
                "loaded OreUI images from {} ({} sprites)",
                dir.display(),
                images.sprites.len()
            );
            Some(images)
        }
        Err(reason) => {
            eprintln!(
                "OreUI bundle at {} unusable ({reason}); OreUI screens draw their own icons",
                dir.display()
            );
            None
        }
    }
}

fn load(dir: &Path) -> Result<OreUiImages, String> {
    let json = read_bounded(&dir.join("atlas.json"), MAX_ATLAS_JSON_BYTES)?;
    let atlases: Vec<AtlasFile> =
        serde_json::from_slice(&json).map_err(|error| format!("atlas.json: {error}"))?;
    let side = OREUI_PAGE_SIDE as usize;
    let mut rgba = vec![0u8; side * side * 4];
    let mut sprites = HashMap::new();
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for atlas in &atlases {
        let (width, height, pixels) = decode(&dir.join(&atlas.name))?;
        if width != atlas.width || height != atlas.height {
            return Err(format!("{} does not match atlas.json", atlas.name));
        }
        if x + width > OREUI_PAGE_SIDE {
            x = 0;
            y += shelf + GUTTER;
            shelf = 0;
        }
        if y + height > OREUI_PAGE_SIDE {
            return Err("atlases do not fit one page".to_owned());
        }
        blit(&mut rgba, side, x, y, &pixels, width, height);
        for (path, rect) in &atlas.coordinates {
            if rect.x + rect.width > width || rect.y + rect.height > height {
                continue;
            }
            let (left, top) = ((x + rect.x) as u16, (y + rect.y) as u16);
            sprites.insert(
                path.clone(),
                [
                    left,
                    top,
                    left + rect.width as u16,
                    top + rect.height as u16,
                ],
            );
        }
        x += width + GUTTER;
        shelf = shelf.max(height);
    }
    Ok(OreUiImages { rgba, sprites })
}

fn blit(target: &mut [u8], side: usize, x: u32, y: u32, source: &[u8], width: u32, height: u32) {
    let row = width as usize * 4;
    for line in 0..height as usize {
        let start = ((y as usize + line) * side + x as usize) * 4;
        target[start..start + row].copy_from_slice(&source[line * row..(line + 1) * row]);
    }
}

fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max {
        return Err(format!("{} is too large", path.display()));
    }
    Ok(bytes)
}

/// A PNG as premultiplied RGBA8, matching the UI pipeline's blending.
fn decode(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let bytes = read_bounded(path, MAX_IMAGE_BYTES)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|error| format!("{}: {error}", path.display()))?
        .into_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    Ok((width, height, pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlases_pack_side_by_side_with_their_sprite_rects() {
        let dir = std::env::temp_dir().join(format!("oreui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(4, 2, image::Rgba([255, 0, 0, 255]))
            .save(dir.join("a.png"))
            .unwrap();
        image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 128]))
            .save(dir.join("b.png"))
            .unwrap();
        std::fs::write(
            dir.join("atlas.json"),
            r#"[{"name":"a.png","width":4,"height":2,"size":1,"coordinates":{"assets/x.png":{"x":1,"y":0,"width":2,"height":2}}},
               {"name":"b.png","width":2,"height":2,"size":1,"coordinates":{"assets/y.png":{"x":0,"y":0,"width":2,"height":2},"assets/bad.png":{"x":1,"y":1,"width":5,"height":5}}}]"#,
        )
        .unwrap();
        let images = load(&dir).unwrap();
        assert_eq!(images.sprites["assets/x.png"], [1, 0, 3, 2]);
        assert_eq!(images.sprites["assets/y.png"], [5, 0, 7, 2]);
        assert!(!images.sprites.contains_key("assets/bad.png"));
        // Premultiplied: half-alpha green halves its channel.
        let pixel = (5 * 4) as usize;
        assert_eq!(&images.rgba[pixel..pixel + 4], &[0, 128, 0, 128]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
