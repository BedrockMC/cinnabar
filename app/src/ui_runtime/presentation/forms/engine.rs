//! Presents a server form through the clean-room JSON-UI engine: the vanilla
//! `ui/*.json` templates resolve against the compiled carrier's catalog, lay out
//! in virtual UI pixels, and their draw nodes become retained UI nodes over the
//! carrier's atlas pages. One virtual pixel is one GUI pixel of the HUD's scale
//! (needs native measurement against Bedrock's own scale-index rule).

use std::{borrow::Cow, cell::RefCell, sync::Arc};

use assets::{RuntimeFontCatalog, RuntimeUiAssets};
use json_ui::{
    Catalog, Context, DataSource, Draw, DrawNode, FormModel, FormRender, LayoutEnv, NineSlice,
    RectOut, TextAlign, TextMeasure, TextureMeta, TextureSource, ViewState, render_form_with,
    render_screen,
};
use ui::{
    SafeArea, TextLayoutCache, TextLayoutRequest, TextShadow, UiNode, UiNodeId, UiScale, UiVisual,
};

use super::super::{FONT_DESIGN_PIXEL_TEXELS, IconRef, TextMetrics, UiPresentationError, rect};
use crate::ui_runtime::{ServerFormIdentity, forms::EngineFrame};

/// Largest wrap width handed to the text layout (logical px), for "no wrap".
const UNWRAPPED_LOGICAL: f64 = 65_536.0;
/// Player preview height relative to its renderer box (needs native measurement).
const PREVIEW_BOX_SCALE: f32 = 2.2;
/// Tooltip placement relative to the pointer and its padding, in virtual px
/// (needs native measurement).
const TOOLTIP_OFFSET: [f32; 2] = [8.0, -12.0];
const TOOLTIP_PAD: f32 = 2.0;
const TOOLTIP_BACKGROUND: [u8; 4] = [16, 0, 16, 224];

pub(crate) struct FormEngine {
    assets: Arc<RuntimeUiAssets>,
    base: Arc<Catalog>,
    catalog: Arc<Catalog>,
    /// Texture page of carrier atlas page 0.
    first_page: u16,
    context: Context,
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
        let base = Arc::new(catalog);
        Self {
            assets,
            catalog: Arc::clone(&base),
            base,
            first_page,
            context: Context::desktop(),
        }
    }

    /// Re-apply a server resource pack's ui files over the vanilla catalog;
    /// an empty set restores the vanilla catalog.
    pub(super) fn set_server_pack(&mut self, files: &[(String, Vec<u8>)]) {
        if files.is_empty() {
            self.catalog = Arc::clone(&self.base);
            return;
        }
        let mut catalog = (*self.base).clone();
        catalog.apply_pack(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        );
        self.catalog = Arc::new(catalog);
    }

    /// Render `model` into `nodes`; `Ok(None)` when its template is missing, so the
    /// caller can fall back to the programmatic dialog.
    pub(super) fn render(
        &self,
        model: &FormModel,
        view: &ViewState,
        identity: ServerFormIdentity,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        self.render_with(
            inputs,
            out,
            ScreenArt::default(),
            Some(identity),
            |env, root| render_form_with(model, &self.catalog, &self.context, root, env, view),
        )
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
        self.render_with(inputs, out, art, None, |env, root| {
            render_screen(reference, &self.catalog, context, data, root, env, view)
        })
    }

    fn render_with(
        &self,
        inputs: EngineInputs<'_>,
        out: EngineOutput<'_>,
        art: ScreenArt<'_>,
        identity: Option<ServerFormIdentity>,
        draw: impl FnOnce(&LayoutEnv, [f64; 2]) -> Option<FormRender>,
    ) -> Result<Option<EngineFrame>, UiPresentationError> {
        let px = inputs.metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let root = [
            f64::from(inputs.content[0] / px),
            f64::from(inputs.content[1] / px),
        ];
        let cache = RefCell::new(inputs.layouts);
        let render = {
            let measure = Measure {
                layouts: &cache,
                font: inputs.font,
                metrics: inputs.metrics,
                px,
                translate: inputs.translate,
            };
            let textures = Sidecars(&self.assets);
            let env = LayoutEnv {
                text: &measure,
                textures: &textures,
            };
            draw(&env, root)
        };
        let Some(render) = render else {
            return Ok(None);
        };
        let layouts = cache.into_inner();
        let mut painter = Painter {
            assets: &self.assets,
            first_page: self.first_page,
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
        for node in render.nodes.iter().chain(out.overlay) {
            painter.paint(node)?;
        }
        Ok(Some(EngineFrame {
            identity,
            hits: render.hits,
            report: render.report,
            cancel_target: render.cancel_target,
            origin: [inputs.safe_area.left(), inputs.safe_area.top()],
            scale: px,
            panel: render
                .root_panel
                .map(|rect| [rect.x, rect.y, rect.w, rect.h]),
        }))
    }
}

/// Caller art the custom renderers draw: the icon table `#item_renderer_data`
/// indexes, the player preview, and the pointer (virtual px) tooltips follow.
#[derive(Clone, Copy, Default)]
pub(super) struct ScreenArt<'a> {
    pub(super) icons: &'a [IconRef],
    pub(super) preview: Option<IconRef>,
    pub(super) pointer: Option<[f32; 2]>,
}

/// Where a render writes its retained nodes, plus caller draw nodes painted on
/// top (e.g. the held stack under the pointer).
pub(super) struct EngineOutput<'a> {
    pub(super) nodes: &'a mut Vec<UiNode>,
    pub(super) next: &'a mut u32,
    pub(super) overlay: &'a [DrawNode],
}

/// A label's text after localization: an exact language key resolves, anything
/// else draws verbatim (vanilla labels localize by default).
fn localized<'a>(text: &'a str, translate: &dyn Fn(&str) -> Option<Arc<str>>) -> Cow<'a, str> {
    if text.is_empty() || text.contains(char::is_whitespace) {
        return Cow::Borrowed(text);
    }
    match translate(text) {
        Some(value) => Cow::Owned(value.to_string()),
        None => Cow::Borrowed(text),
    }
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

fn width_64(logical: f64) -> u32 {
    (logical.clamp(1.0, UNWRAPPED_LOGICAL) * 64.0) as u32
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
        let text = localized(text, self.translate);
        let request =
            self.metrics
                .request(&text, width_64(max_width * f64::from(self.px)), self.font);
        match self.layouts.borrow_mut().layout(request) {
            Ok(layout) => {
                let [w, h] = layout.size_64();
                let px = f64::from(self.px);
                [f64::from(w) / 64.0 / px, f64::from(h) / 64.0 / px]
            }
            Err(_) => [0.0, 0.0],
        }
    }
}

/// Sidecar metadata keyed like the ui json references it; a sidecar-less texture
/// reports its packed pixel size with no nine-slice.
struct Sidecars<'a>(&'a RuntimeUiAssets);

impl TextureSource for Sidecars<'_> {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
        let key = texture_key(path);
        if let Some(sidecar) = self.0.sidecar(key) {
            return Some(TextureMeta {
                base_size: sidecar.base_size.map(f64::from),
                nineslice: sidecar.nineslice.map(|inset| NineSlice {
                    left: f64::from(inset.left),
                    top: f64::from(inset.top),
                    right: f64::from(inset.right),
                    bottom: f64::from(inset.bottom),
                }),
            });
        }
        let placement = self.0.texture(key)?;
        Some(TextureMeta {
            base_size: [f64::from(placement.width), f64::from(placement.height)],
            nineslice: None,
        })
    }
}

/// Ui json sometimes spells a texture with its file extension; the carrier keys
/// drop it.
fn texture_key(path: &str) -> &str {
    for extension in [".png", ".jpg", ".jpeg", ".tga"] {
        if let Some(stem) = path.strip_suffix(extension) {
            return stem;
        }
    }
    path
}

/// Turns engine draw nodes into retained UI nodes, opening a clip group whenever
/// the clip rect changes so draw order is preserved.
struct Painter<'a> {
    assets: &'a RuntimeUiAssets,
    first_page: u16,
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

    /// Bridge the custom renderers container screens use: item icons from the
    /// icon atlas and the durability bar. Others draw nothing yet.
    fn custom(
        &mut self,
        renderer: &str,
        data: &std::collections::BTreeMap<String, serde_json::Value>,
        dest: [f32; 4],
        alpha: impl Fn([u8; 4]) -> [u8; 4],
    ) -> Option<(UiVisual, [f32; 4])> {
        let number = |key: &str| data.get(key).and_then(serde_json::Value::as_f64);
        match renderer {
            "inventory_item_renderer" => {
                let icon = self
                    .art
                    .icons
                    .get(number("#item_renderer_data")? as usize)?;
                Some((
                    UiVisual::Sprite {
                        texture_page: icon.page,
                        uv: icon.uv,
                        color: alpha([255; 4]),
                    },
                    dest,
                ))
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
                Some((
                    UiVisual::Solid {
                        texture_page: self.solid_page,
                        color: alpha(durability_color(fraction)),
                    },
                    fill,
                ))
            }
            // The live model is approximated by the cached preview raster, kept at
            // its aspect and scaled to the renderer's box (needs native measurement).
            "live_player_renderer" | "paper_doll_renderer" => {
                let preview = self.art.preview?;
                let w = f32::from(preview.uv[2].saturating_sub(preview.uv[0]));
                let h = f32::from(preview.uv[3].saturating_sub(preview.uv[1]));
                if w <= 0.0 || h <= 0.0 {
                    return None;
                }
                let height = (dest[3] - dest[1]) * PREVIEW_BOX_SCALE;
                let width = height * w / h;
                let centre = (dest[0] + dest[2]) * 0.5;
                let top = (dest[1] + dest[3]) * 0.5 - height * 0.5 + height * 0.25;
                Some((
                    UiVisual::Sprite {
                        texture_page: preview.page,
                        uv: preview.uv,
                        color: alpha([255; 4]),
                    },
                    [
                        centre - width * 0.5,
                        top,
                        centre + width * 0.5,
                        top + height,
                    ],
                ))
            }
            "hover_text_renderer" => {
                let text = data
                    .get("#hover_text")?
                    .as_str()
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

    /// A solid rect in the current clip group.
    fn solid(&mut self, bounds: [f32; 4], color: [u8; 4]) -> Result<(), UiPresentationError> {
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
            .with_visual(UiVisual::Solid {
                texture_page: self.solid_page,
                color,
            }),
        );
        Ok(())
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
        let alpha = |color: [u8; 4]| {
            let a = (f32::from(color[3]) * node.alpha.clamp(0.0, 1.0)).round() as u8;
            [color[0], color[1], color[2], a]
        };
        // Tooltips ignore the hovered control's clip.
        let clip = match &node.draw {
            Draw::Custom { renderer, .. } if renderer == "hover_text_renderer" => self.screen,
            _ => clip,
        };
        let parent = self.group(clip)?;
        let (visual, bounds) = match &node.draw {
            Draw::Solid { color } => (
                UiVisual::Solid {
                    texture_page: self.solid_page,
                    color: alpha(*color),
                },
                dest,
            ),
            Draw::Sprite { texture, uv, color } => {
                let Some(placement) = self.assets.texture(texture_key(texture)) else {
                    return Ok(());
                };
                let (x, y) = (f32::from(placement.x), f32::from(placement.y));
                let (w, h) = (f32::from(placement.width), f32::from(placement.height));
                let pixel = |base: f32, span: f32, t: f32| (base + span * t).round() as u16;
                (
                    UiVisual::Sprite {
                        texture_page: self.first_page.saturating_add(placement.page),
                        uv: [
                            pixel(x, w, uv.u0),
                            pixel(y, h, uv.v0),
                            pixel(x, w, uv.u1),
                            pixel(y, h, uv.v1),
                        ],
                        color: alpha(*color),
                    },
                    dest,
                )
            }
            Draw::Text {
                text,
                color,
                shadow,
                align,
                scale,
            } => {
                if text.is_empty() {
                    return Ok(());
                }
                let text = localized(text, self.translate);
                let request = scaled_request(
                    &self.metrics,
                    &text,
                    width_64(f64::from(dest[2] - dest[0])),
                    self.font,
                    *scale,
                );
                let Ok(layout) = self.layouts.layout(request) else {
                    return Ok(());
                };
                let width = layout.size_64()[0] as f32 / 64.0;
                let slack = (dest[2] - dest[0] - width).max(0.0);
                let shift = match align {
                    TextAlign::Left => 0.0,
                    TextAlign::Center => slack * 0.5,
                    TextAlign::Right => slack,
                };
                (
                    UiVisual::Text {
                        layout,
                        color: alpha(*color),
                        shadow: if *shadow {
                            self.metrics.shadow()
                        } else {
                            TextShadow::None
                        },
                    },
                    [dest[0] + shift, dest[1], dest[2] + shift, dest[3]],
                )
            }
            Draw::Custom { renderer, data } => match self.custom(renderer, data, dest, alpha) {
                Some(visual) => visual,
                None => return Ok(()),
            },
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
}

/// Durability colour: hue from green (full) to red (worn); needs native measurement.
fn durability_color(fraction: f64) -> [u8; 4] {
    let hue = (fraction / 3.0) * 6.0;
    let x = (1.0 - (hue % 2.0 - 1.0).abs()) as f32;
    let (r, g) = if hue < 1.0 { (1.0, x) } else { (x, 1.0) };
    [(r * 255.0) as u8, (g * 255.0) as u8, 0, 255]
}
