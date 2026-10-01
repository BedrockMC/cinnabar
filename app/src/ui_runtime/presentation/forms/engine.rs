//! Presents a server form through the clean-room JSON-UI engine: the vanilla
//! `ui/*.json` templates resolve against the compiled carrier's catalog, lay out
//! in virtual UI pixels, and their draw nodes become retained UI nodes over the
//! carrier's atlas pages. One virtual pixel is one GUI pixel of the HUD's scale
//! (needs native measurement against Bedrock's own scale-index rule).

use std::{borrow::Borrow, cell::RefCell, sync::Arc};

use assets::{RuntimeFontCatalog, RuntimeUiAssets};
use json_ui::{
    Catalog, Context, DataSource, Draw, DrawNode, FormModel, FormRender, LayoutEnv, RectOut,
    ResolvedControl, ViewState, bind_form, render_bound,
};
use ui::{SafeArea, TextLayoutCache, TextShadow, UiNode, UiNodeId, UiVisual};

use super::super::player_preview::PreviewView;
use super::super::{FONT_DESIGN_PIXEL_TEXELS, IconRef, TextMetrics, UiPresentationError, rect};

mod fill_renderers;
pub(crate) mod hud_renderers;
mod menu_renderers;
mod screen_cache;
mod text_paint;
use super::server_pack::{ServerAtlas, ServerUiPack};
use super::textures::{TextureSet, Textures};
use crate::ui_runtime::{ServerFormIdentity, forms::EngineFrame};
pub(super) use text_paint::active_codes;
use text_paint::{Measure, TextPaint, UNWRAPPED_LOGICAL, scaled_request, width_64};

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
            vanilla,
            base,
            context: super::menu_screens::retail_context(),
            server_pages: Vec::new(),
            server_source: None,
            cache: None,
            passes: [0; 2],
            splash: std::sync::OnceLock::new(),
            screens: screen_cache::ScreenCache::default(),
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

    /// Re-apply a server resource pack's ui files over the vanilla catalog and
    /// the Java HUD pack; an empty set restores the base catalog.
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

    /// Render `model` into `nodes`; `Ok(None)` when its template is missing. The
    /// bound tree and layout are reused until their inputs change.
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
        let frame = render_with(
            art,
            inputs,
            out,
            ScreenArt::default(),
            Some(identity),
            move |env, root| {
                let cache = cache.as_mut()?;
                let fresh = cache
                    .laid
                    .as_ref()
                    .is_some_and(|laid| laid.view == *view && laid.root == root && laid.px == px);
                if !fresh {
                    *passes += 1;
                    let render = render_bound(cache.bound.clone(), root, env, view);
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

    /// Resolve `reference` under `context` in the background ahead of its first open.
    pub(super) fn prewarm(&self, reference: &'static str, context: Context) {
        self.screens.prewarm(reference, &self.catalog, context);
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

    /// Paint what `draw` lays out (given the layout env and root size) over this
    /// engine's textures; `Ok(None)` when it lays out nothing.
    pub(super) fn draw<R: Borrow<FormRender>>(
        &self,
        art: ScreenArt<'_>,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
        draw: impl FnOnce(&LayoutEnv, [f64; 2]) -> Option<R>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        render_with(self.art(), inputs, out, art, None, draw)
    }

    /// Render an allow-listed screen against `data`; `art` backs its custom
    /// renderers (item icons, the player preview, the pointer tooltip).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_screen(
        &self,
        reference: &str,
        data: &DataSource,
        context: &Context,
        view: &ViewState,
        art: ScreenArt<'_>,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
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
                self.progress_bar(data, dest, &alpha);
                None
            }
            "gradient_renderer" => Some((self.gradient(data, &alpha)?, dest)),
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
            _ => None,
        }
    }

    /// A sprite of the texture at `path` (server pack first, then the carrier),
    /// sampling the normalised `uv`; `None` when neither holds it.
    fn sprite(
        &self,
        path: &str,
        uv: json_ui::UvRect,
        color: [u8; 4],
        filter: json_ui::SpriteFilter,
    ) -> Option<UiVisual> {
        let (page, [x, y, w, h]) = self.textures.sprite(path)?;
        let pixel = |base: f32, span: f32, t: f32| (base + span * t).round() as u16;
        let uv = [
            pixel(x, w, uv.u0),
            pixel(y, h, uv.v0),
            pixel(x, w, uv.u1),
            pixel(y, h, uv.v1),
        ];
        let style = u8::from(filter.grayscale) * ui::UI_STYLE_GRAYSCALE
            | u8::from(filter.bilinear) * ui::UI_STYLE_BILINEAR;
        Some(if style == 0 {
            UiVisual::Sprite {
                texture_page: page,
                uv,
                color,
            }
        } else {
            UiVisual::StyledSprite {
                texture_page: page,
                uv,
                color,
                style,
            }
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
        let clip = self.logical(&node.clip);
        let dest = self.logical(&node.dest);
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
            options,
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
                options: options.clone(),
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
            Draw::Sprite {
                texture,
                uv,
                color,
                filter,
            } => {
                let mut uv = *uv;
                if let Some(book) = &node.flip_book {
                    let shift = book.step_u * book.frame(self.art.now) as f32;
                    uv.u0 += shift;
                    uv.u1 += shift;
                }
                let Some(visual) = self.sprite(texture, uv, alpha(*color), *filter) else {
                    return Ok(());
                };
                (visual, dest)
            }
            // Drawn above.
            Draw::Text { .. } => return Ok(()),
            Draw::Custom { renderer, data } if renderer == "hover_text_renderer" => {
                let text = self
                    .art
                    .tooltip
                    .or_else(|| data.get("#hover_text")?.as_str())
                    .filter(|text| !text.is_empty());
                let max_width = data
                    .get("hover_text_max_width")
                    .and_then(serde_json::Value::as_f64);
                return match text {
                    Some(text) => self.tooltip(text, dest, max_width, opacity),
                    None => Ok(()),
                };
            }
            Draw::Custom { renderer, data } => match self.custom(renderer, data, dest, alpha) {
                Some(visual) => visual,
                None => return Ok(()),
            },
        };
        self.push(visual, bounds)
    }
}
