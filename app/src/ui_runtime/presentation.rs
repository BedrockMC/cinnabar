use std::{fmt, sync::Arc};

use assets::{RuntimeFontCatalog, RuntimeHudCatalog, RuntimeIconCatalog};
use bevy::{
    camera::Camera,
    math::Vec3,
    prelude::{Camera3d, GlobalTransform, Query, Res, ResMut, Resource, Time, With},
    time::Real,
    window::{PrimaryWindow, Window},
};
use render::{
    ActorSkinPixels, ChunkRenderQueue, ChunkUploadAcknowledgements, VisibilityDiagnostics,
    VisibilityDiagnosticsInput,
};
use render::{UiRenderInput, UiRenderScene, UiRenderStats, UiRenderTextureArray};
use sha2::{Digest, Sha256};

use ui::{
    DpiScale, ObfuscationGlyphs, SafeArea, TextEffects, TextLayoutCache, UiNode, UiNodeId, UiPoint,
    UiRect, UiScale, UiTree, UiVisual,
};

use super::{UiRuntime, render_adapter::UiRenderViewport};
use crate::{
    camera::CameraSettingsAuthority,
    runtime::{
        shutdown::record_fatal_error,
        visibility::CaveVisibilityCache,
        world::{ClientWorld, WorldStreamFramePoll},
    },
    ui_runtime::{item_facts, render_adapter::adapt_ui_draw_list},
};

mod debug_overlay;
mod dynamic_textures;
pub(crate) mod forms;
mod hud_layout;
pub(crate) mod inventory_pointer;
mod inventory_tooltip;
mod item_sprite;
mod item_viewmodel;
mod menu;
mod menu_artwork;
mod menu_scroll;
pub(crate) use menu_artwork::BUILT_IN_TITLE;
mod nametags;
mod player_preview;
mod primitives;
mod publish;
mod retained_hud;
pub(crate) mod screens;
mod session_glyphs;
mod session_icons;
pub(crate) use forms::ServerUiPack;
pub(crate) use session_glyphs::SessionGlyphSheets;
pub(crate) use session_icons::{MAX_SESSION_ICON_SIDE, SessionIcon, SessionIcons};
mod startup;
mod text_metrics;
mod texture_atlas;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "dormant until presentation owns an exact walk-distance query cadence"
    )
)]
mod viewmodel_bob;

use crate::menu::{MenuAction, MenuView};
pub(crate) use debug_overlay::DebugLines;
pub(crate) use forms::{BedHit, ChatHit, LoadingStage, drive_menu_panorama};
pub(crate) use hud_layout::HudFrame;
use hud_layout::{HudGeometry, HudLayout, gui_scale};
use primitives::{bounded_visible_text, rect, resolve_chat_line};
#[cfg(test)]
pub(crate) use publish::refresh_hud_frame;
pub(crate) use publish::{observe_mount_jump_input, platform_safe_area_insets, publish_ui_runtime};
use retained_hud::{BelowNameAnchor, PresentedScoreboardCache, ScoreboardOwnerNameAuthority};
use startup::{StartupPresentationState, StartupReadinessInput};
use text_metrics::{
    FONT_DESIGN_PIXEL_TEXELS, TEXT_BASELINE_64, TEXT_LINE_HEIGHT_64, TEXT_SHADOW_OFFSET_64,
    TextMetrics,
};
pub(crate) use texture_atlas::IconRef;
use texture_atlas::{
    HudTexturePages, font_texture_array, font_texture_array_with_hud_and_icons,
    font_texture_array_with_optional_hud,
};

const TEXT_CACHE_ENTRIES: usize = 1_024;
const TEXT_CACHE_BYTES: usize = 8 * 1024 * 1024;
const MAX_PRESENTED_TEXT_BYTES: usize = 512;
#[derive(Debug)]
pub enum UiPresentationError {
    InvalidFontTexture,
    Geometry(ui::GeometryError),
    Text(ui::TextError),
    Tree(ui::UiError),
    Adapter(super::render_adapter::UiRenderAdapterError),
    Render(render::UiRenderReject),
}

impl fmt::Display for UiPresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "UI presentation failed: {self:?}")
    }
}

impl std::error::Error for UiPresentationError {}

#[derive(Resource)]
pub struct UiPresentationRuntime {
    font: Arc<RuntimeFontCatalog>,
    /// The startup font without the session's glyph sheets.
    base_font: Arc<RuntimeFontCatalog>,
    textures: Arc<UiRenderTextureArray>,
    texture_session: Option<u64>,
    blank_dynamic_page: render::UiTexturePage,
    solid_texture_page: u16,
    hud_textures: Option<HudTexturePages>,
    icon_catalog: Option<Arc<RuntimeIconCatalog>>,
    icon_refs: Option<Box<[IconRef]>>,
    layouts: TextLayoutCache,
    obfuscation: ObfuscationGlyphs, // same-width pools for the per-frame §k swap
    revision: u64,
    last_input: Option<UiRenderInput>, // last built frame; see `stabilize_revision`
    scoreboard: PresentedScoreboardCache,
    scoreboard_owner_names: ScoreboardOwnerNameAuthority,
    debug_lines: Option<DebugLines>,
    /// Java GUI-scale preference: `None`/0 selects the auto rule.
    gui_scale_preference: Option<u8>,
    /// Platform safe-area insets in logical px, applied to the HUD geometry,
    /// the retained tree layout, and the render viewport alike.
    safe_area: SafeArea,
    /// Item facts and camera state refreshed immediately before each build.
    hud_frame: HudFrame,
    /// Last logged skip/odd-data counters, so changes surface exactly once.
    last_hud_diagnostics: crate::ui_runtime::gameplay_hud::GameplayHudDiagnostics,
    /// World-projected below-name score anchors for the current frame.
    below_name_anchors: Vec<BelowNameAnchor>,
    /// World-projected nametags for players without a below-name score.
    nametag_anchors: Vec<nametags::NametagAnchor>,
    /// Stable reserved logical page for the optional preview raster.
    player_preview_page: Option<u16>,
    player_preview_source_hash: Option<[u8; 32]>,
    player_preview_pose: Option<player_preview::PlayerPreviewPose>,
    /// How the UI last asked to show the model, and the idle sway it was drawn at.
    player_preview_view: player_preview::PreviewView,
    player_preview_drawn: Option<(
        player_preview::PreviewView,
        f32,
        player_preview::PreviewEquipment,
    )>,
    player_preview_bob: f32,
    /// Worn armor and the held item the model shows, and where armor art comes from.
    player_preview_gear: player_preview::PreviewEquipment,
    equipment_catalog: Option<Arc<assets::RuntimeEquipmentCatalog>>,
    player_preview_pixels: Option<player_preview::PlayerPreviewRasters>,
    preview_dirty: bool,
    player_preview_icon: Option<IconRef>,
    left_hand_icon: Option<IconRef>,
    right_hand_icon: Option<IconRef>,
    held_viewmodel_source: Option<IconRef>,
    offhand_viewmodel_source: Option<IconRef>,
    held_viewmodel_icon: Option<IconRef>,
    offhand_viewmodel_icon: Option<IconRef>,
    menu_artwork_paths: Vec<(String, u32)>,
    /// This frame's clock in seconds, for menu animations painted over cached layouts.
    menu_seconds: f64,
    /// Engine textures too big for a server page, keyed by texture path.
    menu_artwork_oversized: Vec<(String, Arc<[u8]>)>,
    menu_artwork: menu_artwork::MenuArtworkAtlas,
    menu_artwork_dirty: bool,
    session_icons: session_icons::SessionIconPage,
    session_glyphs: session_glyphs::SessionGlyphPages,
    /// Identifiers already logged as iconless.
    missing_icons: std::sync::Mutex<std::collections::HashSet<String>>,
    /// The hotbar last logged: each slot's identifier and whether it had an icon.
    logged_hotbar: [Option<(Arc<str>, bool)>; 9],
    menu_view: Option<MenuView>,
    menu_hit_targets: Vec<(MenuAction, UiRect)>,
    menu_scrolls: menu_scroll::MenuScrolls,
    form_presentation: forms::FormPresentation,
    /// Window-space rect of the sign editor's Done button in the last build.
    loading_stage: Option<LoadingStage>,
    startup: StartupPresentationState,
}

impl UiPresentationRuntime {
    pub fn new(font: Arc<RuntimeFontCatalog>) -> Result<Self, UiPresentationError> {
        Self::with_optional_hud(font, None)
    }

    pub fn with_hud(
        font: Arc<RuntimeFontCatalog>,
        hud: Arc<RuntimeHudCatalog>,
    ) -> Result<Self, UiPresentationError> {
        Self::with_optional_assets(font, Some(hud), None)
    }

    pub fn with_hud_and_icons(
        font: Arc<RuntimeFontCatalog>,
        hud: Arc<RuntimeHudCatalog>,
        icons: Arc<RuntimeIconCatalog>,
    ) -> Result<Self, UiPresentationError> {
        Self::with_optional_assets(font, Some(hud), Some(icons))
    }

    fn with_optional_assets(
        font: Arc<RuntimeFontCatalog>,
        hud: Option<Arc<RuntimeHudCatalog>>,
        icons: Option<Arc<RuntimeIconCatalog>>,
    ) -> Result<Self, UiPresentationError> {
        let (textures, solid_texture_page, hud_textures, icon_refs) =
            match (hud.as_deref(), icons.as_deref()) {
                (Some(hud), None) => {
                    let (textures, solid_texture_page, hud_textures) =
                        font_texture_array_with_optional_hud(&font, Some(hud))?;
                    (textures, solid_texture_page, hud_textures, None)
                }
                (None, None) => {
                    let (textures, solid_texture_page) = font_texture_array(&font)?;
                    (textures, solid_texture_page, None, None)
                }
                (hud, icons) => font_texture_array_with_hud_and_icons(&font, hud, icons)?,
            };
        let textures = Arc::new(textures);
        Ok(Self {
            obfuscation: ObfuscationGlyphs::from_catalog(&font),
            base_font: Arc::clone(&font),
            font,
            blank_dynamic_page: textures.pages()[textures.dynamic_start()].clone(),
            textures,
            texture_session: None,
            solid_texture_page,
            hud_textures,
            icon_catalog: icons,
            icon_refs,
            layouts: TextLayoutCache::new(TEXT_CACHE_ENTRIES, TEXT_CACHE_BYTES),
            revision: 0,
            last_input: None,
            scoreboard: PresentedScoreboardCache::default(),
            scoreboard_owner_names: ScoreboardOwnerNameAuthority::default(),
            debug_lines: None,
            gui_scale_preference: None,
            safe_area: SafeArea::ZERO,
            hud_frame: HudFrame::default(),
            last_hud_diagnostics: Default::default(),
            below_name_anchors: Vec::new(),
            nametag_anchors: Vec::new(),
            player_preview_page: None,
            player_preview_source_hash: None,
            player_preview_pose: None,
            player_preview_view: player_preview::PreviewView::default(),
            player_preview_drawn: None,
            player_preview_bob: 0.0,
            player_preview_gear: player_preview::PreviewEquipment::default(),
            equipment_catalog: None,
            player_preview_pixels: None,
            preview_dirty: false,
            player_preview_icon: None,
            left_hand_icon: None,
            right_hand_icon: None,
            held_viewmodel_source: None,
            offhand_viewmodel_source: None,
            held_viewmodel_icon: None,
            offhand_viewmodel_icon: None,
            menu_artwork_paths: Vec::new(),
            menu_seconds: 0.0,
            menu_artwork_oversized: Vec::new(),
            menu_artwork: menu_artwork::MenuArtworkAtlas::default(),
            // The title logo loads before any service art arrives.
            menu_artwork_dirty: true,
            session_icons: session_icons::SessionIconPage::default(),
            session_glyphs: session_glyphs::SessionGlyphPages::default(),
            missing_icons: Default::default(),
            logged_hotbar: Default::default(),
            menu_view: None,
            menu_hit_targets: Vec::new(),
            menu_scrolls: Default::default(),
            form_presentation: forms::FormPresentation::default(),
            loading_stage: None,
            startup: StartupPresentationState::default(),
        })
    }

    pub(crate) fn set_loading_stage(&mut self, stage: Option<LoadingStage>) {
        self.loading_stage = stage;
    }

    /// Updates the cached corner avatar. The raster is regenerated and the UI
    /// texture array is replaced only when the authoritative skin or pose
    /// changes; normal camera/HUD frames reuse the same GPU texture.
    pub(crate) fn set_player_preview_skin(
        &mut self,
        skin: Option<&[u8]>,
        pose: player_preview::PlayerPreviewPose,
    ) {
        let default_skin = render::default_actor_skin_rgba8();
        let skin = skin
            .filter(|pixels| pixels.len() == render::STANDARD_SKIN_BYTES)
            .unwrap_or(default_skin.as_ref());
        let source_hash: [u8; 32] = Sha256::digest(skin).into();
        let drawn = (
            self.player_preview_view,
            self.player_preview_bob,
            self.player_preview_gear.clone(),
        );
        if self.player_preview_source_hash == Some(source_hash)
            && self.player_preview_pose == Some(pose)
            && self.player_preview_drawn.as_ref() == Some(&drawn)
        {
            return;
        }
        self.player_preview_pixels = Some(player_preview::PlayerPreviewRasters {
            preview: player_preview::render(skin, pose, drawn.0, drawn.1, &drawn.2),
            left_hand: player_preview::render_hand(skin, pose, true),
            right_hand: player_preview::render_hand(skin, pose, false),
        });
        self.player_preview_drawn = Some(drawn);
        self.player_preview_source_hash = Some(source_hash);
        self.player_preview_pose = Some(pose);
        self.preview_dirty = true;
        self.rebuild_dynamic_textures();
    }

    pub(crate) const fn player_preview_icon(&self) -> Option<IconRef> {
        self.player_preview_icon
    }

    pub(crate) const fn player_hand_icons(&self) -> (Option<IconRef>, Option<IconRef>) {
        (self.left_hand_icon, self.right_hand_icon)
    }

    pub(super) fn rebuild_dynamic_textures(&mut self) {
        dynamic_textures::rebuild(self);
    }

    /// Service art at `paths`, plus the engine's oversized textures, on the art pages.
    pub(crate) fn sync_menu_artwork(&mut self, paths: Vec<(String, u32)>) {
        let oversized = self.oversized_ui_textures();
        let same_oversized = oversized.len() == self.menu_artwork_oversized.len()
            && oversized
                .iter()
                .zip(&self.menu_artwork_oversized)
                .all(|(a, b)| a.0 == b.0);
        if self.menu_artwork_paths == paths && same_oversized {
            return;
        }
        self.menu_artwork_paths = paths;
        self.menu_artwork_oversized = oversized;
        self.menu_artwork_dirty = true;
        self.rebuild_dynamic_textures();
    }

    pub(crate) fn menu_artwork_icon(&self, path: &str) -> Option<IconRef> {
        self.menu_artwork.refs.get(path).copied()
    }

    pub(crate) fn set_menu_view(&mut self, view: Option<MenuView>) {
        self.menu_view = view;
    }

    pub(crate) fn hit_test_menu(&self, position: UiPoint) -> Option<MenuAction> {
        self.menu_hit_targets
            .iter()
            .rev()
            .find_map(|(action, bounds)| bounds.contains(position).then_some(*action))
    }

    fn with_optional_hud(
        font: Arc<RuntimeFontCatalog>,
        hud: Option<Arc<RuntimeHudCatalog>>,
    ) -> Result<Self, UiPresentationError> {
        Self::with_optional_assets(font, hud, None)
    }

    /// Selects a fixed Java GUI scale (1..=4); `None` or 0 restores auto.
    pub fn set_gui_scale_preference(&mut self, preference: Option<u8>) {
        self.gui_scale_preference = preference.filter(|value| *value > 0);
    }

    /// Binds the platform's reported safe-area insets (logical px). Every
    /// subsequent frame lays out inside the inset viewport and clips renders
    /// to it; viewports too inset for the fixed HUD fail closed to no HUD.
    pub fn set_safe_area(&mut self, safe_area: SafeArea) {
        self.safe_area = safe_area;
    }

    #[cfg(test)]
    pub(crate) fn hud_frame(&self) -> &HudFrame {
        &self.hud_frame
    }

    pub(crate) fn hud_frame_mut(&mut self) -> &mut HudFrame {
        &mut self.hud_frame
    }

    fn set_below_name_anchors(&mut self, anchors: impl IntoIterator<Item = BelowNameAnchor>) {
        self.below_name_anchors.clear();
        self.below_name_anchors.extend(
            anchors
                .into_iter()
                .take(retained_hud::MAX_PRESENTED_BELOW_NAME_ROWS),
        );
    }

    fn set_nametag_anchors(&mut self, anchors: Vec<nametags::NametagAnchor>) {
        self.nametag_anchors = anchors;
    }

    /// Retained text-layout cache entries, exposed for the bounded-memory
    /// steady-state witnesses.
    #[cfg(test)]
    pub(crate) fn layout_cache_len(&self) -> usize {
        self.layouts.len()
    }

    pub fn build(
        &mut self,
        runtime: &UiRuntime,
        now_millis: u64,
        physical_size: [u32; 2],
        dpi_scale: DpiScale,
    ) -> Result<UiRenderInput, UiPresentationError> {
        dynamic_textures::observe_session(self, runtime.session_id());
        session_icons::observe(self, runtime.session_icons());
        self.observe_server_ui(runtime.server_ui());
        session_glyphs::observe(self, runtime.session_glyphs());
        let logical_width = physical_size[0] as f32 / dpi_scale.get();
        let logical_height = physical_size[1] as f32 / dpi_scale.get();
        let metrics =
            TextMetrics::for_viewport(physical_size, dpi_scale, self.gui_scale_preference);
        // The gameplay HUD lays out in Java GUI pixels; it fails closed to no
        // HUD when the safe viewport cannot contain the fixed-width hotbar.
        let safe_area = self.safe_area;
        let hud_geometry = self.hud_textures.as_ref().and_then(|_| {
            HudGeometry::new(
                physical_size,
                dpi_scale.get(),
                safe_area,
                self.gui_scale_preference,
            )
        });
        let viewport = rect(0.0, 0.0, logical_width, logical_height)?;
        // Root nodes lay out relative to the safe content rect; the retained
        // tree translates them by the safe-area origin.
        let content_width = (logical_width - safe_area.left() - safe_area.right()).max(0.0);
        let content_height = (logical_height - safe_area.top() - safe_area.bottom()).max(0.0);
        let mut nodes = Vec::new();
        let mut next_id = 1u32;
        let menu_visible = self.menu_view.is_some();
        if !menu_visible
            && let Some(hud_textures) = self.hud_textures.as_ref()
            && let Some(geometry) = hud_geometry
        {
            let mut frame = self.hud_frame.clone();
            frame.now_millis = now_millis;
            let mut layout = HudLayout::new(
                &mut nodes,
                &mut next_id,
                hud_textures,
                &mut self.layouts,
                &self.font,
                self.solid_texture_page,
                geometry,
            )?;
            layout.append(runtime, &frame)?;
        }

        let inventory_open = runtime.inventory_open();
        if !inventory_open && !menu_visible {
            self.append_engine_hud(
                runtime,
                &mut nodes,
                &mut next_id,
                metrics,
                [content_width, content_height],
                now_millis,
            )?;
        }

        if !inventory_open && !menu_visible {
            self.append_debug_overlay(&mut nodes, &mut next_id, metrics, content_width)?;
            nametags::append_nametag_nodes(
                &mut nodes,
                &mut next_id,
                &mut self.layouts,
                &self.font,
                metrics,
                self.solid_texture_page,
                &self.nametag_anchors,
                [content_width, content_height],
            )?;
            retained_hud::append_below_name_nodes(
                &mut nodes,
                &mut next_id,
                &mut self.layouts,
                &self.font,
                metrics,
                self.solid_texture_page,
                content_width,
                content_height,
                &self.below_name_anchors,
            )?;
        }

        if !menu_visible && !inventory_open {
            self.append_bed_screen(
                runtime,
                &mut nodes,
                &mut next_id,
                metrics,
                [content_width, content_height],
                now_millis,
            )?;
        }
        if !menu_visible && !inventory_open && runtime.chat_focused() {
            self.append_chat_screen(
                runtime,
                &mut nodes,
                &mut next_id,
                metrics,
                [content_width, content_height],
                now_millis,
            )?;
        } else {
            self.close_chat_screen();
        }

        self.menu_seconds = now_millis as f64 / 1_000.0;
        let menu_hit_targets = self.append_menu(
            runtime,
            &mut nodes,
            &mut next_id,
            metrics,
            content_width,
            content_height,
        )?;

        if !menu_visible && let Some(stage) = self.loading_stage {
            // An opaque cover under the loading screen: no partial terrain or
            // HUD leaks through while the world settles.
            nodes.push(
                UiNode::new(
                    UiNodeId::new(next_id),
                    None,
                    rect(0.0, 0.0, logical_width, logical_height)?,
                )
                .with_visual(UiVisual::Solid {
                    texture_page: self.solid_texture_page,
                    color: [8, 10, 14, 255],
                }),
            );
            next_id = next_id.saturating_add(1);
            self.append_loading_screen(
                runtime,
                stage,
                &mut nodes,
                &mut next_id,
                metrics,
                [content_width, content_height],
            )?;
        }

        self.append_server_form(
            runtime,
            &mut nodes,
            &mut next_id,
            metrics,
            content_width,
            content_height,
        )?;
        self.append_sign_editor(
            runtime,
            &mut nodes,
            &mut next_id,
            metrics,
            content_width,
            content_height,
            now_millis,
        )?;
        if !menu_visible {
            self.append_toast_screen(
                runtime,
                &mut nodes,
                &mut next_id,
                metrics,
                [content_width, content_height],
                now_millis,
            )?;
        }
        self.sync_server_ui_pages();
        let mut tree = UiTree::new(nodes).map_err(UiPresentationError::Tree)?;
        tree.layout(viewport, UiScale::default(), safe_area)
            .map_err(UiPresentationError::Tree)?;
        let draw_list = tree
            .build_draw_list_with(TextEffects {
                obfuscation_seed: now_millis,
                obfuscation: Some(&self.obfuscation),
            })
            .map_err(UiPresentationError::Tree)?;
        let input = adapt_ui_draw_list(
            &draw_list,
            Arc::clone(&self.textures),
            UiRenderViewport {
                physical_size,
                dpi_scale,
                safe_area,
            },
        )
        .map_err(UiPresentationError::Adapter)?;
        let input = self.stabilize_revision(input);
        self.menu_hit_targets = menu_hit_targets;
        Ok(input)
    }
}

#[cfg(test)]
pub(crate) mod tests;
