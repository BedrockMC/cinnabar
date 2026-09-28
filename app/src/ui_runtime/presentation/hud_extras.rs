//! Optional HUD extras carrier: hardcore hearts packed into one extra static UI page.

use std::{path::Path, sync::Arc};

use assets::{HUD_EXTRA_SIDE, HudExtraRole, HudExtras, HudTextureRole};
use render::{UiRenderTextureArray, UiTexturePage};
use sha2::{Digest, Sha256};

use super::UiPresentationRuntime;

const HUD_EXTRAS_FILENAME: &str = "vanilla-v1.mcbehxt";
const HUD_EXTRAS_COMPILE_COMMAND: &str = "make hud-extras-assets";
const PAGE_SIDE: u32 = 256;
const GUTTER: u32 = 1;

/// Page and pixel UVs of every hardcore heart sprite.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HardcoreHearts {
    page: u16,
    uv: [[u16; 4]; HudExtraRole::ALL.len()],
}

impl HardcoreHearts {
    /// Page and UV of the hardcore sprite replacing a standard heart foreground role.
    pub(crate) fn sprite(&self, role: HudTextureRole) -> Option<(u16, [u16; 4])> {
        let extra = HudExtraRole::for_heart(role)?;
        Some((self.page, self.uv[extra as usize]))
    }
}

/// Loads the optional carrier next to the world carrier; absence keeps the standard hearts.
pub(crate) fn load_optional(world_asset_path: &Path) -> Option<(HudExtras, [u8; 32])> {
    let path = world_asset_path.with_file_name(HUD_EXTRAS_FILENAME);
    let decoded = std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| assets::decode_hud_extras(&bytes).map_err(|error| error.to_string()));
    match decoded {
        Ok(loaded) => {
            eprintln!("loaded HUD extras from {}", path.display());
            Some(loaded)
        }
        Err(error) => {
            eprintln!(
                "HUD extras unavailable at {} ({error}); hardcore worlds use the standard hearts; build with {HUD_EXTRAS_COMPILE_COMMAND}",
                path.display()
            );
            None
        }
    }
}

impl UiPresentationRuntime {
    /// Appends the extras page ahead of the dynamic pages; call before any session is observed.
    pub(crate) fn install_hud_extras(&mut self, extras: &HudExtras, identity: [u8; 32]) {
        let Some((catalog, hearts)) = build(&self.textures, extras, identity) else {
            eprintln!("HUD extras do not fit the UI texture budget; using the standard hearts");
            return;
        };
        self.blank_dynamic_page = catalog.pages()[catalog.dynamic_start()].clone();
        self.textures = Arc::new(catalog);
        self.hardcore_hearts = Some(hearts);
    }
}

fn build(
    old: &UiRenderTextureArray,
    extras: &HudExtras,
    identity: [u8; 32],
) -> Option<(UiRenderTextureArray, HardcoreHearts)> {
    let stride = HUD_EXTRA_SIDE + GUTTER * 2;
    let side = HUD_EXTRA_SIDE as usize;
    let mut rgba8 = vec![0u8; (PAGE_SIDE * PAGE_SIDE * 4) as usize];
    let mut uv = [[0u16; 4]; HudExtraRole::ALL.len()];
    for role in HudExtraRole::ALL {
        let index = role as u32;
        let cell_left = index * stride;
        if cell_left + stride > PAGE_SIDE {
            return None;
        }
        let image = extras.rgba8(role);
        for y in 0..stride {
            let source_y = (y.saturating_sub(GUTTER) as usize).min(side - 1);
            for x in 0..stride {
                let source_x = (x.saturating_sub(GUTTER) as usize).min(side - 1);
                let from = (source_y * side + source_x) * 4;
                let to = ((y * PAGE_SIDE + cell_left + x) * 4) as usize;
                rgba8[to..to + 4].copy_from_slice(&image[from..from + 4]);
            }
        }
        let left = u16::try_from(cell_left + GUTTER).ok()?;
        let top = u16::try_from(GUTTER).ok()?;
        uv[index as usize] = [left, top, left + HUD_EXTRA_SIDE as u16, top + HUD_EXTRA_SIDE as u16];
    }
    let start = old.dynamic_start();
    let mut pages = old.pages().to_vec();
    pages.insert(
        start,
        UiTexturePage::owned([PAGE_SIDE, PAGE_SIDE], Arc::from(rgba8)).ok()?,
    );
    let mut source = Sha256::new();
    source.update(b"ui-hud-extras-v1");
    source.update(old.static_identity());
    source.update(identity);
    let catalog =
        UiRenderTextureArray::with_source_identity(pages, start + 1, source.finalize().into())
            .ok()?;
    let page = u16::try_from(start).ok()?;
    Some((catalog, HardcoreHearts { page, uv }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hearts_expose_only_standard_heart_roles() {
        let hearts = HardcoreHearts {
            page: 3,
            uv: [[1, 1, 10, 10]; HudExtraRole::ALL.len()],
        };
        assert_eq!(hearts.sprite(HudTextureRole::HeartFull), Some((3, [1, 1, 10, 10])));
        assert_eq!(hearts.sprite(HudTextureRole::HungerFull), None);
    }
}
