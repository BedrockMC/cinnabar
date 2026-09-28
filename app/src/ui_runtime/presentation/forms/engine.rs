//! Presents a server form through the clean-room JSON-UI engine: the vanilla
//! `ui/*.json` templates resolve against the compiled carrier's catalog, lay out
//! in virtual UI pixels, and their draw nodes become retained UI nodes over the
//! carrier's atlas pages. One virtual pixel is one GUI pixel of the HUD's scale
//! (needs native measurement against Bedrock's own scale-index rule).

use std::{borrow::Cow, cell::RefCell, sync::Arc};

use assets::{RuntimeFontCatalog, RuntimeUiAssets};
use json_ui::{
    Catalog, Context, Draw, DrawNode, FormModel, LayoutEnv, NineSlice, RectOut, TextAlign,
    TextMeasure, TextureMeta, TextureSource, ViewState, render_form_with,
};
use ui::{
    SafeArea, TextLayoutCache, TextLayoutRequest, TextShadow, UiNode, UiNodeId, UiScale, UiVisual,
};

use super::super::{FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationError, rect};
use crate::ui_runtime::{ServerFormIdentity, forms::EngineFrame};

/// Largest wrap width handed to the text layout (logical px), for "no wrap".
const UNWRAPPED_LOGICAL: f64 = 65_536.0;

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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render(
        &self,
        model: &FormModel,
        view: &ViewState,
        identity: ServerFormIdentity,
        inputs: EngineInputs<'_>,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
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
            render_form_with(model, &self.catalog, &self.context, root, &env, view)
        };
        let Some(render) = render else {
            return Ok(None);
        };
        let layouts = cache.into_inner();
        let mut painter = Painter {
            assets: &self.assets,
            first_page: self.first_page,
            solid_page: inputs.solid_page,
            layouts,
            font: inputs.font,
            metrics: inputs.metrics,
            px,
            translate: inputs.translate,
            nodes,
            next,
            clip: None,
        };
        for node in &render.nodes {
            painter.paint(node)?;
        }
        Ok(Some(EngineFrame {
            identity,
            hits: render.hits,
            report: render.report,
            cancel_target: render.cancel_target,
            origin: [inputs.safe_area.left(), inputs.safe_area.top()],
            scale: px,
        }))
    }
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
            // Custom renderers (item icons, paper doll) are bridged by the screens
            // that use them; a form has none.
            Draw::Custom { .. } => return Ok(()),
        };
        let parent = self.group(clip)?;
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
