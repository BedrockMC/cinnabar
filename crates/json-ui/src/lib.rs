//! Clean-room parser, resolver, and layout engine for vanilla Bedrock JSON-UI.
//!
//! The pipeline reads the on-disk `ui/*.json` (tolerant JSON5), applies `@base`
//! inheritance, substitutes `$var`/global references, evaluates `ignored` and
//! `variables[]` conditionals, and records factory mappings into a concrete
//! [`ResolvedControl`] tree. From there [`layout`] evaluates the size/offset
//! expressions ([`expr`]) against a virtual root size and positions every control,
//! and [`emit`] flattens the placed tree into layer-ordered draw commands,
//! nine-slicing sprites from their texture sidecars ([`sidecar`]). [`bind`] resolves
//! `#bindings` against a screen data source and expands factory collections and
//! grids, [`widgets`] drives the engine-owned control states from a caller-held
//! [`ViewState`], [`input`] reports interactive regions and mappings, [`pack`]
//! overlays server resource-pack ui over the vanilla catalog, and [`form`]
//! renders a decoded server form through its vanilla template.

mod anim;
mod bind;
mod catalog;
mod emit;
mod env;
mod expr;
mod form;
mod hud;
mod input;
mod json5;
mod layout;
mod localize;
mod merge;
mod pack;
mod predicate;
mod resolve;
mod screens;
mod sidecar;
mod state;
mod tree;
mod widgets;

use std::collections::BTreeMap;

use serde_json::Value;

pub use anim::{Chain, Fade, Step, StepKind, fade_factor, fade_factor_at};
pub use bind::{
    CollectionItem, ControlLibrary, DataSource, EmptyLibrary, FactoryItem, bind, bind_shared,
    scoped_key,
};
pub use catalog::{Catalog, LoadError, RawControl};
pub use emit::{Draw, DrawNode, RectOut, SpriteQuad, TextAlign, UvRect, emit, nine_slice};
pub use env::Env;
pub use expr::{
    AxisContext, ExprError, Length, Resolved, Term, Unit, length_from_value, parse_length,
};
pub use form::{
    ActionElement, ActionForm, ButtonImage, CachedLibrary, CatalogLibrary, CustomElement,
    CustomForm, FormButton, FormModel, FormRender, ModalForm, ResolveCache, bind_form,
    form_context, form_data_source, form_screen_cancel, form_template, render_bound, render_form,
    render_form_with,
};
pub use hud::{
    BossBar, CROSSHAIR_SCREEN, HUD_SCREEN, HudModel, HudSlot, HudTitle, Sidebar, Timed, hud_clocks,
    hud_context, hud_data_source,
};
pub use input::{
    HitKind, HitRegion, focus_order, global_mapping, hit_regions, hit_test, region_rect,
    scroll_target,
};
pub use layout::{LaidOut, LayoutEnv, Rect, TextMeasure, TextureSource, layout, layout_with};
pub use localize::localize_text;
pub use predicate::{Bindings, Scalar};
pub use resolve::Resolver;
pub use screens::{ENGINE_SCREENS, ScreenRender, is_engine_screen, render_screen};
pub use sidecar::{NineSlice, TextureMeta, parse_texture_meta};
pub use state::{LayoutReport, ScrollMetrics, ViewState};
pub use tree::{ControlRef, Factory, ResolvedControl};

/// Screen context: the compile-time flags (`$desktop_screen`, `$touch`, …) and any
/// extra variables that gate `ignored`/`variables[]` selection and `$var` values.
#[derive(Clone, Debug, Default)]
pub struct Context {
    vars: BTreeMap<String, Value>,
}

impl Context {
    /// An empty context; every conditional depending on an unset flag is treated
    /// leniently (kept for `ignored`, skipped for `requires`).
    pub fn empty() -> Self {
        Self::default()
    }

    /// The desktop screen context used by this tranche's fixtures.
    pub fn desktop() -> Self {
        Self::empty()
            .with_flag("desktop_screen", true)
            .with_flag("pocket_screen", false)
            .with_flag("touch", false)
    }

    /// Set a boolean flag (stored under `name`, without a `$`).
    pub fn with_flag(mut self, name: &str, value: bool) -> Self {
        self.vars.insert(name.to_owned(), Value::Bool(value));
        self
    }

    /// Set an arbitrary variable value.
    pub fn with_var(mut self, name: &str, value: Value) -> Self {
        self.vars.insert(name.to_owned(), value);
        self
    }

    fn root_env(&self, catalog: &Catalog) -> Env {
        let mut env = Env::new();
        for (name, value) in catalog.globals() {
            env.set(name.clone(), value.clone());
        }
        for (name, value) in &self.vars {
            env.set(name.clone(), value.clone());
        }
        env
    }
}

/// The outcome of a resolution: the tree (absent only when the reference is
/// unknown) and the diagnostics gathered along the way.
#[derive(Debug)]
pub struct Resolution {
    pub control: Option<ResolvedControl>,
    pub diagnostics: Vec<String>,
}

/// Resolve a `namespace.name` reference against `catalog` in `context`.
pub fn resolve(catalog: &Catalog, reference: &str, context: &Context) -> Resolution {
    let root = context.root_env(catalog);
    let mut resolver = Resolver::new(catalog);
    let control = match reference.split_once('.') {
        Some((namespace, name)) => {
            let resolved = resolver.resolve(namespace, name, &root);
            if resolved.is_none() {
                resolver.note(format!("unknown control `{reference}`"));
            }
            resolved
        }
        None => {
            resolver.note(format!("reference `{reference}` is not `namespace.name`"));
            None
        }
    };
    Resolution {
        control,
        diagnostics: resolver.into_diagnostics(),
    }
}
