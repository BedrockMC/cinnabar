//! Presents a server form through the clean-room JSON-UI engine: the vanilla
//! `ui/*.json` templates resolve against the compiled carrier's catalog, lay out
//! in virtual UI pixels, and their draw nodes become retained UI nodes over the
//! carrier's atlas pages. One virtual pixel is one GUI pixel of the HUD's scale
//! (needs native measurement against Bedrock's own scale-index rule).

use std::{
    borrow::{Borrow, Cow},
    cell::RefCell,
    sync::Arc,
};

use assets::{RuntimeFontCatalog, RuntimeUiAssets};
use json_ui::{
    Catalog, Context, DataSource, Draw, DrawNode, FormModel, FormRender, LayoutEnv, RectOut,
    ResolvedControl, TextAlign, TextMeasure, ViewState, bind_form, render_bound_gated,
};
use ui::{
    SafeArea, TextLayoutCache, TextLayoutRequest, TextShadow, UiNode, UiNodeId, UiScale, UiVisual,
};

use super::super::player_preview::PreviewView;
use super::super::{FONT_DESIGN_PIXEL_TEXELS, IconRef, TextMetrics, UiPresentationError, rect};

pub(crate) mod hud_renderers;
mod menu_renderers;
pub(super) mod screen_cache;
use super::server_pack::{ServerAtlas, ServerUiPack};
use super::textures::{TextureSet, Textures};
use crate::ui_runtime::{ServerFormIdentity, forms::EngineFrame};

/// Largest wrap width handed to the text layout (logical px), for "no wrap".
const UNWRAPPED_LOGICAL: f64 = 65_536.0;
/// Tooltip offset from the pointer and padding, virtual px (needs native measurement).
const TOOLTIP_OFFSET: [f32; 2] = [8.0, -12.0];
const TOOLTIP_PAD: f32 = 2.0;
const TOOLTIP_BACKGROUND: [u8; 4] = [16, 0, 16, 224];

pub(crate) struct FormEngine {
    assets: Arc<RuntimeUiAssets>,
    /// The carrier's vanilla catalog, before the built-in Java HUD pack.
    vanilla: Arc<Catalog>,
    /// Vanilla under the built-in Java HUD pack: the catalog with no server pack.
    base: Arc<Catalog>,
    catalog: Arc<Catalog>,
    context: Context,
    /// Where texture paths draw from, including the on-demand server atlas.
    pub(super) textures: TextureSet,
    /// The atlas page images last handed to the dynamic pages.
    pub(super) server_pages: Vec<render::UiTexturePage>,
    /// The runtime pack last applied, compared by identity.
    server_source: Option<Arc<ServerUiPack>>,
    /// The last form's bound tree and laid-out output, reused while unchanged.
    pub(super) cache: Option<FormCache>,
    /// Resolve+bind and layout passes run, for cache tests and profiling.
    pub(super) passes: [usize; 2],
    /// The title splash, picked once per launch.
    splash: std::sync::OnceLock<Option<String>>,
    screens: screen_cache::ScreenCache,
}

pub(super) struct FormCache {
    model: FormModel,
    catalog: Arc<Catalog>,
    bound: ResolvedControl,
    laid: Option<LaidForm>,
    /// The screen's Escape target; flattening the screen per frame deep-clones pack controls.
    screen_cancel: Option<String>,
}

struct LaidForm {
    view: ViewState,
    root: [f64; 2],
    px: f32,
    render: FormRender,
}

/// Borrowed texture sources a paint reads.
#[derive(Clone, Copy)]
struct Art<'a> {
    assets: &'a RuntimeUiAssets,
    set: &'a TextureSet,
}

/// Everything a render borrows from the presentation runtime for one frame.
pub(super) struct EngineInputs<'a> {
    pub(super) layouts: &'a mut TextLayoutCache,
    pub(super) font: &'a RuntimeFontCatalog,
    pub(super) metrics: TextMetrics,
    pub(super) solid_page: u16,
    pub(super) safe_area: SafeArea,
    pub(super) content: [f32; 2],
    pub(super) translate: &'a dyn Fn(&str) -> Option<Arc<str>>,
}

impl FormEngine {
    pub(super) fn new(assets: Arc<RuntimeUiAssets>, catalog: Catalog, first_page: u16) -> Self {
        let vanilla = Arc::new(catalog);
        let base = Arc::new(with_java_hud(&vanilla, &Default::default()));
        Self {
            textures: TextureSet::new(&assets, first_page),
            assets,
            catalog: Arc::clone(&base),
            screens: screen_cache::ScreenCache::resolving_settings(&base),
            vanilla,
            base,
            context: super::menu_screens::retail_context(),
            server_pages: Vec::new(),
            server_source: None,
            cache: None,
            passes: [0; 2],
            splash: std::sync::OnceLock::new(),
        }
    }

    fn art(&self) -> Art<'_> {
        Art {
            assets: &self.assets,
            set: &self.textures,
        }
    }

    /// Records `pack` as the applied source; `true` when it differs from the last.
    pub(super) fn take_server_source(&mut self, pack: Option<&Arc<ServerUiPack>>) -> bool {
        let same = match (&self.server_source, pack) {
            (Some(current), Some(next)) => Arc::ptr_eq(current, next),
            (None, None) => true,
            _ => false,
        };
        self.server_source = pack.cloned();
        !same
    }

    /// Which catalog forms resolve against, for the render-path log.
    pub(super) fn catalog_label(&self) -> String {
        if Arc::ptr_eq(&self.catalog, &self.base) {
            "vanilla catalog with the Java HUD pack".to_owned()
        } else {
            let notes = self
                .catalog
                .diagnostics()
                .len()
                .saturating_sub(self.vanilla.diagnostics().len());
            format!("server pack overlay, {notes} pack diagnostics")
        }
    }

    /// Install a server texture atlas whose pages start at texture page `first`.
    pub(super) fn set_server_atlas(&mut self, atlas: ServerAtlas, first: u16) {
        self.textures.set_atlas(atlas, first);
    }

    /// The atlas page images when they changed since the last call.
    pub(super) fn take_server_pages(&mut self) -> Option<&[render::UiTexturePage]> {
        let atlas = self.textures.atlas_mut();
        if !atlas.take_dirty() {
            return None;
        }
        self.server_pages = atlas.images().to_vec();
        Some(&self.server_pages)
    }

    /// The last form's sprite textures, and those resolving to no source.
    #[cfg(test)]
    pub(super) fn drawn_sprites(&self) -> (Vec<String>, Vec<String>) {
        let atlas = self.textures.lock();
        let view = Textures {
            assets: &self.assets,
            set: &self.textures,
            atlas: &atlas,
            images: None,
        };
        let mut drawn: Vec<String> = self
            .cache
            .iter()
            .flat_map(|cache| cache.laid.iter())
            .flat_map(|laid| laid.render.nodes.iter())
            .filter_map(|node| match &node.draw {
                Draw::Sprite { texture, .. } => Some(view.canonical(texture).into_owned()),
                _ => None,
            })
            .collect();
        drawn.sort();
        drawn.dedup();
        let missing = drawn
            .iter()
            .filter(|key| view.sprite(key).is_none())
            .cloned()
            .collect();
        (drawn, missing)
    }

    /// Apply a server pack's ui files over vanilla and the Java HUD pack; none restores the base.
    pub(super) fn set_server_pack(&mut self, layers: &[Vec<(String, Vec<u8>)>]) {
        if layers.iter().all(Vec::is_empty) {
            self.catalog = Arc::clone(&self.base);
            return;
        }
        let touched = layers
            .iter()
            .flat_map(|files| {
                self.vanilla.overlay_namespaces(
                    files
                        .iter()
                        .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
                )
            })
            .collect();
        let mut catalog = with_java_hud(&self.vanilla, &touched);
        for files in layers {
            catalog.apply_pack(
                files
                    .iter()
                    .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
            );
        }
        for note in catalog
            .diagnostics()
            .iter()
            .skip(self.vanilla.diagnostics().len())
        {
            bevy::log::debug!(note, "server ui pack");
        }
        self.catalog = Arc::new(catalog);
    }

    /// Render `model`; `Ok(None)` without its template. Layout holds until model or scroll change.
    pub(super) fn render(
        &mut self,
        model: &FormModel,
        view: &ViewState,
        identity: ServerFormIdentity,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        let current = self.cache.as_ref().is_some_and(|cache| {
            cache.model == *model && Arc::ptr_eq(&cache.catalog, &self.catalog)
        });
        if !current {
            self.passes[0] += 1;
            self.cache = bind_form(model, &self.catalog, &self.context).map(|bound| FormCache {
                model: model.clone(),
                catalog: Arc::clone(&self.catalog),
                bound,
                laid: None,
                screen_cancel: json_ui::form_screen_cancel(&self.catalog),
            });
        }
        let px = inputs.metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let art = Art {
            assets: &self.assets,
            set: &self.textures,
        };
        let screen_cancel = self
            .cache
            .as_ref()
            .and_then(|cache| cache.screen_cancel.clone());
        let (cache, passes) = (&mut self.cache, &mut self.passes[1]);
        let screen_art = ScreenArt {
            view: Some(view),
            ..ScreenArt::default()
        };
        let frame = render_with(
            art,
            inputs,
            out,
            screen_art,
            Some(identity),
            move |env, root| {
                let cache = cache.as_mut()?;
                let fresh = cache.laid.as_ref().is_some_and(|laid| {
                    laid.view.scroll == view.scroll && (laid.root, laid.px) == (root, px)
                });
                if !fresh {
                    *passes += 1;
                    let measures = &mut Default::default();
                    let render = render_bound_gated(cache.bound.clone(), root, env, view, measures);
                    cache.laid = Some(LaidForm {
                        view: view.clone(),
                        root,
                        px,
                        render,
                    });
                }
                cache.laid.as_ref().map(|laid| &laid.render)
            },
        )?;
        Ok(frame.map(|mut frame| {
            frame.cancel_target = frame.cancel_target.or(screen_cancel);
            frame
        }))
    }

    pub(super) fn assets(&self) -> &RuntimeUiAssets {
        &self.assets
    }

    /// Lay `screen` out in the background ahead of its first open.
    pub(super) fn prepare(&self, screen: screen_cache::Prepared) {
        self.screens.prepare(screen, self);
    }

    pub(super) fn splash(&self, translate: &dyn Fn(&str) -> Option<Arc<str>>) -> Option<&str> {
        self.splash
            .get_or_init(|| menu_renderers::pick_splash(&self.assets, translate))
            .as_deref()
    }

    pub(super) fn catalog(&self) -> &Arc<Catalog> {
        &self.catalog
    }

    pub(super) fn context(&self) -> &Context {
        &self.context
    }

    /// Paint what `draw` lays out over this engine's textures; `Ok(None)` when it lays out nothing.
    pub(super) fn draw<R: Borrow<FormRender>>(
        &self,
        art: ScreenArt<'_>,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
        draw: impl FnOnce(&LayoutEnv, [f64; 2]) -> Option<R>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        render_with(self.art(), inputs, out, art, None, draw)
    }

    /// Render an allow-listed screen against `data` under `view`; `art` backs its custom renderers.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_screen<'a>(
        &self,
        reference: &str,
        data: &DataSource,
        context: &Context,
        view: &'a ViewState,
        mut art: ScreenArt<'a>,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        art.view = Some(view);
        let px = inputs.metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let language = (inputs.translate)("menu.play");
        render_with(self.art(), inputs, out, art, None, |env, root| {
            let key = screen_cache::ScreenKey {
                reference,
                catalog: &self.catalog,
                context,
                data,
                view,
                root,
                px,
                language,
            };
            self.screens.render(key, env)
        })
    }
}

/// `vanilla` under the built-in Java HUD pack, less its files for namespaces in
/// `withdrawn` (restyled by a server pack authored against vanilla); no Mojang footer.
fn with_java_hud(vanilla: &Catalog, withdrawn: &std::collections::BTreeSet<String>) -> Catalog {
    let mut catalog = vanilla.clone();
    let kept = super::hud::JAVA_HUD_PACK
        .iter()
        .filter(|(_, namespace, _)| !withdrawn.contains(*namespace))
        .map(|(path, _, bytes)| (*path, *bytes));
    catalog.apply_pack(kept);
    catalog.apply_pack(
        [(
            "ui/cinnabar_title.json",
            menu_renderers::TITLE_PANEL_OVERLAY,
        )]
        .into_iter()
        .chain(menu_renderers::NO_COPYRIGHT_OVERLAYS),
    );
    catalog
}

fn render_with<R: Borrow<FormRender>>(
    textures: Art<'_>,
    inputs: EngineInputs<'_>,
    out: EngineOutput<'_>,
    art: ScreenArt<'_>,
    identity: Option<ServerFormIdentity>,
    draw: impl FnOnce(&LayoutEnv, [f64; 2]) -> Option<R>,
) -> Result<Option<EngineFrame>, UiPresentationError> {
    let px = inputs.metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
    let root = [
        f64::from(inputs.content[0] / px),
        f64::from(inputs.content[1] / px),
    ];
    let cache = RefCell::new(inputs.layouts);
    let render = {
        let atlas = textures.set.lock();
        let view = Textures {
            assets: textures.assets,
            set: textures.set,
            atlas: &atlas,
            images: art.images,
        };
        let measure = Measure {
            layouts: &cache,
            font: inputs.font,
            metrics: inputs.metrics,
            px,
            translate: inputs.translate,
        };
        let env = LayoutEnv {
            text: &measure,
            textures: &view,
        };
        draw(&env, root)
    };
    let Some(render) = render else {
        return Ok(None);
    };
    let render = render.borrow();
    let layouts = cache.into_inner();
    let mut atlas = textures.set.lock();
    // Only what this screen draws needs to be resident.
    // Many nodes share a texture; each path resolves once.
    let paths: std::collections::HashSet<&str> = render
        .nodes
        .iter()
        .chain(out.overlay)
        .filter_map(|node| match &node.draw {
            Draw::Sprite { texture, .. } => Some(texture.as_str()),
            _ => None,
        })
        .chain(
            art.hud
                .into_iter()
                .flat_map(hud_renderers::HudPaint::textures),
        )
        .collect();
    let drawn = Textures {
        assets: textures.assets,
        set: textures.set,
        atlas: &atlas,
        images: art.images,
    }
    .atlas_keys(paths.into_iter());
    atlas.require(drawn.iter().map(String::as_str));
    let mut painter = Painter {
        textures: Textures {
            assets: textures.assets,
            set: textures.set,
            atlas: &atlas,
            images: art.images,
        },
        solid_page: inputs.solid_page,
        art,
        screen: [0.0, 0.0, inputs.content[0], inputs.content[1]],
        layouts,
        font: inputs.font,
        metrics: inputs.metrics,
        px,
        translate: inputs.translate,
        nodes: out.nodes,
        next: out.next,
        clip: None,
    };
    let view = art.view;
    for node in render.nodes.iter().chain(out.overlay) {
        if view.is_none_or(|view| node.shown(view)) {
            painter.paint(node)?;
        }
    }
    Ok(Some(EngineFrame {
        identity,
        hits: render.hits.clone(),
        report: render.report.clone(),
        cancel_target: render.cancel_target.clone(),
        origin: [inputs.safe_area.left(), inputs.safe_area.top()],
        scale: px,
        panel: render
            .root_panel
            .map(|rect| [rect.x, rect.y, rect.w, rect.h]),
    }))
}

/// Caller art the custom renderers draw: `#item_renderer_data` icons, the player preview,
/// the tooltip pointer (virtual px), the fade clock (s), HUD state, artwork and gamerpic.
#[derive(Clone, Copy, Default)]
pub(super) struct ScreenArt<'a> {
    pub(super) icons: &'a [IconRef],
    /// Icons an `#item_id_aux` renderer names, by that value.
    pub(super) id_aux: &'a [(i64, IconRef)],
    /// The interaction state gated nodes ([`json_ui::render_bound_gated`]) paint under.
    pub(super) view: Option<&'a ViewState>,
    /// Text a shown hover tooltip draws instead of its bound `#hover_text`.
    pub(super) tooltip: Option<&'a str>,
    /// Where a drawn player renderer records how it wants the model posed.
    pub(super) preview_view: Option<&'a std::cell::Cell<Option<PreviewView>>>,
    pub(super) preview: Option<IconRef>,
    pub(super) pointer: Option<[f32; 2]>,
    pub(super) now: f64,
    /// Creation times that fades naming a clock read instead of their own.
    pub(super) clocks: Option<&'a std::collections::BTreeMap<String, f64>>,
    pub(super) hud: Option<&'a hud_renderers::HudPaint>,
    pub(super) images: Option<&'a std::collections::HashMap<String, IconRef>>,
    pub(super) portrait: Option<IconRef>,
    pub(super) splash: Option<&'a str>,
}

/// Where a render writes its retained nodes, plus caller nodes painted on top (the held stack).
pub(super) struct EngineOutput<'a> {
    pub(super) nodes: &'a mut Vec<UiNode>,
    pub(super) next: &'a mut u32,
    pub(super) overlay: &'a [DrawNode],
}

/// A label's text after vanilla localization; empty lines drop as the vanilla label drops them.
fn localized<'a>(text: &'a str, translate: &dyn Fn(&str) -> Option<Arc<str>>) -> Cow<'a, str> {
    let text = json_ui::localize_text(text, translate);
    if text.contains("\n\n") || text.starts_with('\n') || text.ends_with('\n') {
        let lines: Vec<&str> = text.split('\n').filter(|line| !line.is_empty()).collect();
        return Cow::Owned(lines.join("\n"));
    }
    text
}

fn scaled_request<'a>(
    metrics: &TextMetrics,
    text: &'a str,
    width_64: u32,
    font: &'a RuntimeFontCatalog,
    factor: f32,
) -> TextLayoutRequest<'a> {
    let mut request = metrics.request(text, width_64, font);
    if factor != 1.0
        && let Ok(scale) = UiScale::new(metrics.scale.get() * factor)
    {
        request.scale = scale;
    }
    request
}

/// Rounded up, so text laid out at its own measured width does not wrap.
fn width_64(logical: f64) -> u32 {
    (logical.clamp(1.0, UNWRAPPED_LOGICAL) * 64.0).ceil() as u32
}

struct Measure<'a, 'b> {
    layouts: &'b RefCell<&'a mut TextLayoutCache>,
    font: &'a RuntimeFontCatalog,
    metrics: TextMetrics,
    px: f32,
    translate: &'a dyn Fn(&str) -> Option<Arc<str>>,
}

impl TextMeasure for Measure<'_, '_> {
    fn extent(&self, text: &str) -> [f64; 2] {
        self.wrapped(text, UNWRAPPED_LOGICAL / f64::from(self.px))
    }

    fn wrapped(&self, text: &str, max_width: f64) -> [f64; 2] {
        if text.is_empty() {
            return [0.0, 0.0];
        }
        let request =
            self.metrics
                .request(text, width_64(max_width * f64::from(self.px)), self.font);
        match self.layouts.borrow_mut().layout(request) {
            Ok(layout) => {
                let [w, h] = layout.size_64();
                let px = f64::from(self.px);
                [f64::from(w) / 64.0 / px, f64::from(h) / 64.0 / px]
            }
            Err(_) => [0.0, 0.0],
        }
    }

    fn localize<'t>(&self, text: &'t str) -> Cow<'t, str> {
        localized(text, self.translate)
    }
}

/// Turns draw nodes into retained UI nodes, opening a clip group per clip change to keep order.
struct Painter<'a> {
    textures: Textures<'a>,
    solid_page: u16,
    art: ScreenArt<'a>,
    /// The whole content area, the clip for unclipped tooltips.
    screen: [f32; 4],
    layouts: &'a mut TextLayoutCache,
    font: &'a RuntimeFontCatalog,
    metrics: TextMetrics,
    px: f32,
    translate: &'a dyn Fn(&str) -> Option<Arc<str>>,
    nodes: &'a mut Vec<UiNode>,
    next: &'a mut u32,
    clip: Option<([f32; 4], UiNodeId)>,
}

impl Painter<'_> {
    fn logical(&self, rect: &RectOut) -> [f32; 4] {
        let px = self.px;
        [
            rect.x as f32 * px,
            rect.y as f32 * px,
            (rect.x + rect.w) as f32 * px,
            (rect.y + rect.h) as f32 * px,
        ]
    }

    fn id(&mut self) -> UiNodeId {
        let id = UiNodeId::new(*self.next);
        *self.next = self.next.saturating_add(1);
        id
    }

    /// The clip group for `clip`, reusing the current one when it matches.
    fn group(&mut self, clip: [f32; 4]) -> Result<UiNodeId, UiPresentationError> {
        if let Some((current, id)) = self.clip
            && current == clip
        {
            return Ok(id);
        }
        let id = self.id();
        self.nodes.push(
            UiNode::new(id, None, rect(clip[0], clip[1], clip[2], clip[3])?)
                .with_clip_children(true),
        );
        self.clip = Some((clip, id));
        Ok(id)
    }

    /// Bridge the custom renderers screens use: item icons from the icon atlas,
    /// the durability bar, the player preview, tooltips, and the HUD's native
    /// renderers. Others draw nothing yet.
    fn custom(
        &mut self,
        renderer: &str,
        data: &std::collections::BTreeMap<String, serde_json::Value>,
        dest: [f32; 4],
        alpha: impl Fn([u8; 4]) -> [u8; 4],
    ) -> Option<(UiVisual, [f32; 4])> {
        let number = |key: &str| data.get(key).and_then(serde_json::Value::as_f64);
        if let Some(hud) = self.art.hud
            && hud_renderers::paint(self, hud, renderer, data, dest, &alpha)
        {
            return None;
        }
        match renderer {
            "inventory_item_renderer" => {
                let icon = match number("#item_renderer_data") {
                    Some(index) => self.art.icons.get(index as usize)?,
                    None => {
                        let key = number("#item_id_aux")? as i64;
                        &self.art.id_aux.iter().find(|(id, _)| *id == key)?.1
                    }
                };
                Some((icon.visual(alpha([255; 4])), dest))
            }
            "progress_bar_renderer" => {
                if data.get("#touch_progress_bar_visible") != Some(&serde_json::Value::Bool(true)) {
                    return None;
                }
                let total = number("#progress_bar_total_amount").filter(|total| *total > 0.0)?;
                let fraction = (number("#progress_bar_current_amount")? / total).clamp(0.0, 1.0);
                // Track then fill; the fill hue sweeps green to red with wear.
                let track = [dest[0], dest[1], dest[2], dest[3] + (dest[3] - dest[1])];
                self.solid(track, alpha([0, 0, 0, 255])).ok()?;
                let width = (dest[2] - dest[0]) * fraction as f32;
                let fill = [dest[0], dest[1], dest[0] + width, dest[3]];
                // A loading bar names its colour; an item's durability bar sweeps its hue.
                let color = data
                    .get("primary_color")
                    .and_then(json_ui::color_value)
                    .unwrap_or_else(|| durability_color(fraction));
                Some((
                    UiVisual::Solid {
                        texture_page: self.solid_page,
                        color: alpha(color),
                    },
                    fill,
                ))
            }
            // Messaging art is drawn as its first frame.
            "animated_gif_renderer" => {
                let path = data.get("#gif_path")?.as_str()?;
                let image = self.art.images?.get(path)?;
                let opacity = number("#alpha").unwrap_or(1.0).clamp(0.0, 1.0);
                let tint = alpha([255, 255, 255, (255.0 * opacity) as u8]);
                Some((
                    UiVisual::Sprite {
                        texture_page: image.page,
                        uv: image.uv,
                        color: tint,
                    },
                    dest,
                ))
            }
            "profile_image_renderer" => {
                let portrait = self.art.portrait?;
                Some((
                    UiVisual::Sprite {
                        texture_page: portrait.page,
                        uv: portrait.uv,
                        color: alpha([255; 4]),
                    },
                    dest,
                ))
            }
            "live_player_renderer" | "paper_doll_renderer" => {
                self.player_preview(renderer, data, dest, &alpha)
            }
            "splash_text_renderer" => {
                self.splash(dest, &alpha);
                None
            }
            "name_tag_renderer" => self.name_tag(data, dest, &alpha),
            "hover_text_renderer" => {
                let text = self
                    .art
                    .tooltip
                    .or_else(|| data.get("#hover_text")?.as_str())
                    .filter(|text| !text.is_empty())?;
                self.tooltip(text, dest).ok().flatten()
            }
            _ => None,
        }
    }

    /// A tooltip box beside the pointer (or the hovered control) holding `text`.
    fn tooltip(
        &mut self,
        text: &str,
        dest: [f32; 4],
    ) -> Result<Option<(UiVisual, [f32; 4])>, UiPresentationError> {
        let anchor = self.art.pointer.map_or([dest[2], dest[1]], |point| {
            [point[0] * self.px, point[1] * self.px]
        });
        let request = self
            .metrics
            .request(text, width_64(UNWRAPPED_LOGICAL), self.font);
        let Ok(layout) = self.layouts.layout(request) else {
            return Ok(None);
        };
        let [w, h] = layout.size_64().map(|size| size as f32 / 64.0);
        let pad = TOOLTIP_PAD * self.px;
        let x = (anchor[0] + TOOLTIP_OFFSET[0] * self.px).min(self.screen[2] - w - pad * 2.0);
        let y = (anchor[1] + TOOLTIP_OFFSET[1] * self.px).max(0.0);
        self.solid(
            [x, y, x + w + pad * 2.0, y + h + pad * 2.0],
            TOOLTIP_BACKGROUND,
        )?;
        Ok(Some((
            UiVisual::Text {
                layout,
                color: [255; 4],
                shadow: self.metrics.shadow(),
            },
            [x + pad, y + pad, x + pad + w, y + pad + h],
        )))
    }

    /// A label's text, one node per source line so each aligns on its own. Only
    /// whole lines that fit the label's height draw (the first always does), as
    /// the vanilla label drops lines past its height.
    fn text(
        &mut self,
        text: &str,
        dest: [f32; 4],
        clip: [f32; 4],
        style: TextPaint,
    ) -> Result<(), UiPresentationError> {
        let text = if style.localize {
            localized(text, self.translate)
        } else {
            Cow::Borrowed(text)
        };
        let mut top = dest[1];
        let mut carry = String::new();
        for (index, line) in text.split('\n').enumerate() {
            let source = format!("{carry}{line}");
            carry = active_codes(&source);
            if line.is_empty() {
                continue;
            }
            let request = scaled_request(
                &self.metrics,
                &source,
                width_64(f64::from(dest[2] - dest[0])),
                self.font,
                style.scale,
            );
            let Ok(layout) = self.layouts.layout(request) else {
                continue;
            };
            let [width, height] = layout.size_64().map(|size| size as f32 / 64.0);
            let pitch = height / f32::from(layout.line_count().max(1));
            let room = ((dest[3] - top) / pitch + 0.01).floor().max(0.0);
            if index > 0 && room < 1.0 {
                break;
            }
            let shown = room.clamp(1.0, f32::from(layout.line_count().max(1)));
            let bottom = (top + shown * pitch).min(clip[3]);
            let line_clip = [clip[0], clip[1], clip[2], bottom];
            if line_clip[3] <= line_clip[1] {
                break;
            }
            let slack = (dest[2] - dest[0] - width).max(0.0);
            let shift = match style.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => slack * 0.5,
                TextAlign::Right => slack,
            };
            let parent = self.group(line_clip)?;
            let id = self.id();
            let x = dest[0] + shift;
            self.nodes.push(
                UiNode::new(
                    id,
                    Some(parent),
                    rect(
                        x - line_clip[0],
                        top - line_clip[1],
                        x + width.max(1.0) - line_clip[0],
                        top + height - line_clip[1],
                    )?,
                )
                .with_visual(UiVisual::Text {
                    layout,
                    color: style.color,
                    shadow: style.shadow,
                }),
            );
            top += height;
        }
        Ok(())
    }

    /// A sprite of the texture at `path` (server pack first, then the carrier),
    /// sampling the normalised `uv`; `None` when neither holds it.
    fn sprite(&self, path: &str, uv: json_ui::UvRect, color: [u8; 4]) -> Option<UiVisual> {
        let (page, [x, y, w, h]) = self.textures.sprite(path)?;
        let pixel = |base: f32, span: f32, t: f32| (base + span * t).round() as u16;
        Some(UiVisual::Sprite {
            texture_page: page,
            uv: [
                pixel(x, w, uv.u0),
                pixel(y, h, uv.v0),
                pixel(x, w, uv.u1),
                pixel(y, h, uv.v1),
            ],
            color,
        })
    }

    /// Push `visual` at `bounds` into the current clip group.
    fn push(&mut self, visual: UiVisual, bounds: [f32; 4]) -> Result<(), UiPresentationError> {
        let Some((clip, parent)) = self.clip else {
            return Ok(());
        };
        let id = self.id();
        self.nodes.push(
            UiNode::new(
                id,
                Some(parent),
                rect(
                    bounds[0] - clip[0],
                    bounds[1] - clip[1],
                    bounds[2] - clip[0],
                    bounds[3] - clip[1],
                )?,
            )
            .with_visual(visual),
        );
        Ok(())
    }

    /// A solid rect in the current clip group.
    fn solid(&mut self, bounds: [f32; 4], color: [u8; 4]) -> Result<(), UiPresentationError> {
        let visual = UiVisual::Solid {
            texture_page: self.solid_page,
            color,
        };
        self.push(visual, bounds)
    }

    fn paint(&mut self, node: &DrawNode) -> Result<(), UiPresentationError> {
        let (dest, clip) = node.animated_rects(self.art.now, self.art.clocks);
        let clip = self.logical(&clip);
        let dest = self.logical(&dest);
        if clip[2] <= clip[0]
            || clip[3] <= clip[1]
            || dest[2] <= dest[0]
            || dest[3] <= dest[1]
            || node.alpha <= 0.0
        {
            return Ok(());
        }
        let fade = match self.art.clocks {
            Some(clocks) => json_ui::fade_factor_at(&node.fades, self.art.now, clocks),
            None => json_ui::fade_factor(&node.fades, self.art.now),
        };
        let opacity = node.alpha * fade;
        if opacity <= 0.0 {
            return Ok(());
        }
        let alpha = |color: [u8; 4]| {
            let a = (f32::from(color[3]) * opacity.clamp(0.0, 1.0)).round() as u8;
            [color[0], color[1], color[2], a]
        };
        // Tooltips ignore the hovered control's clip.
        let clip = match &node.draw {
            Draw::Custom { renderer, .. } if renderer == "hover_text_renderer" => self.screen,
            _ => clip,
        };
        if let Draw::Text {
            text,
            color,
            shadow,
            align,
            scale,
            localize,
        } = &node.draw
        {
            let style = TextPaint {
                color: alpha(*color),
                shadow: if *shadow {
                    self.metrics.shadow()
                } else {
                    TextShadow::None
                },
                align: *align,
                scale: *scale,
                localize: *localize,
            };
            return self.text(text, dest, clip, style);
        }
        self.group(clip)?;
        let (visual, bounds) = match &node.draw {
            Draw::Solid { color } => (
                UiVisual::Solid {
                    texture_page: self.solid_page,
                    color: alpha(*color),
                },
                dest,
            ),
            Draw::Sprite { texture, uv, color } => {
                let mut uv = *uv;
                if let Some(book) = &node.flip_book {
                    let shift = book.step_u * book.frame(self.art.now) as f32;
                    uv.u0 += shift;
                    uv.u1 += shift;
                }
                let Some(visual) = self.sprite(texture, uv, alpha(*color)) else {
                    return Ok(());
                };
                (visual, dest)
            }
            // Drawn above, one node per line.
            Draw::Text { .. } => return Ok(()),
            Draw::Custom { renderer, data } => match self.custom(renderer, data, dest, alpha) {
                Some(visual) => visual,
                None => return Ok(()),
            },
        };
        self.push(visual, bounds)
    }
}

#[derive(Clone, Copy)]
struct TextPaint {
    color: [u8; 4],
    shadow: TextShadow,
    align: TextAlign,
    scale: f32,
    localize: bool,
}

/// The format codes in force at the end of `text`, to open the next line with.
pub(super) fn active_codes(text: &str) -> String {
    let mut codes = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '§' {
            continue;
        }
        match characters.next() {
            Some('r') => codes.clear(),
            Some(code @ ('0'..='9' | 'a'..='w')) => {
                codes.push('§');
                codes.push(code);
            }
            _ => {}
        }
    }
    codes
}

/// Durability colour: hue from green (full) to red (worn); needs native measurement.
fn durability_color(fraction: f64) -> [u8; 4] {
    let hue = (fraction / 3.0) * 6.0;
    let x = (1.0 - (hue % 2.0 - 1.0).abs()) as f32;
    let (r, g) = if hue < 1.0 { (1.0, x) } else { (x, 1.0) };
    [(r * 255.0) as u8, (g * 255.0) as u8, 0, 255]
}
