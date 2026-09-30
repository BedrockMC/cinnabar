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

pub use anim::{Chain, Fade, FlipBook, Step, StepKind, fade_factor, fade_factor_at};
pub use bind::{
    CollectionItem, ControlLibrary, DataSource, EmptyLibrary, FactoryItem, bind, bind_reporting,
    bind_shared, scoped_key,
};
pub use catalog::{Catalog, LoadError, RawControl};
pub use emit::{
    Draw, DrawNode, RectOut, SpriteQuad, StateGate, TextAlign, UvRect, color_value, emit,
    emit_gated, nine_slice,
};
pub use env::Env;
pub use expr::{
    AxisContext, ExprError, Length, Resolved, Term, Unit, length_from_value, parse_length,
};
pub use form::{
    ActionElement, ActionForm, ButtonImage, CachedLibrary, CatalogLibrary, CustomElement,
    CustomForm, FormButton, FormModel, FormRender, ModalForm, ResolveCache, bind_form,
    form_context, form_data_source, form_factory_id, form_screen_cancel, form_template,
    render_bound, render_bound_gated, render_form, render_form_with,
};
pub use hud::{
    BossBar, CROSSHAIR_SCREEN, HUD_SCREEN, HudModel, HudSlot, HudTitle, Sidebar, Timed, hud_clocks,
    hud_context, hud_data_source,
};
pub use input::{
    HitKind, HitRegion, focus_order, global_mapping, hit_regions, hit_test, region_rect,
    scroll_target,
};
pub use layout::{
    LaidOut, LayoutEnv, MeasureCache, Rect, TextMeasure, TextureSource, layout, layout_with,
};
pub use localize::localize_text;
pub use predicate::{Bindings, Scalar};
pub use resolve::Resolver;
pub use screens::{
    ENGINE_SCREENS, ScreenRender, bind_screen, is_engine_screen, render_screen, resolve_screen,
};
pub use sidecar::{NineSlice, TextureMeta, parse_texture_meta};
pub use state::{LayoutReport, ScrollMetrics, ViewState};
pub use tree::{ControlRef, Factory, ResolvedControl};

/// Screen context: the compile-time flags (`$desktop_screen`, `$touch`, …) and any
/// extra variables that gate `ignored`/`variables[]` selection and `$var` values.
#[derive(Clone, Debug, Default, PartialEq)]
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

    pub fn retail(macos: bool) -> Self {
        let platform: &[(&str, bool)] = &[
            ("win10_edition", !macos),
            ("microsoft_os", !macos),
            ("ms_platform", !macos),
            ("osx_edition", macos),
            ("apple_os", macos),
        ];
        let constant: &[(&str, bool)] = &[
            ("is_desktop", true),
            ("mouse", true),
            ("is_publish", true),
            ("test_infrastructure_disabled", true),
            ("new_video_settings", true),
            ("is_improve_input_response_platform_supported", true),
            ("is_xboxlive_enabled", true),
            ("is_realms_enabled", true),
            ("is_seeds_enabled", true),
            ("is_creative_enabled", true),
            ("is_multiplayer_enabled", true),
            ("is_packs_enabled", true),
            ("is_server_enabled", true),
            ("is_store_enabled", true),
            ("file_picking_supported", true),
            ("supports_clipboard_set", true),
            ("supports_add_friend", true),
            ("supports_xbl_achievements", true),
            // Channel flags: this is the release app, not Preview.
            ("pre_release", false),
            ("beta_build", false),
            ("is_preview_app", false),
            ("trial", false),
            ("education_edition", false),
            ("store_disabled", false),
            ("creator_build", false),
            ("pocket_edition", false),
            ("console_edition", false),
            ("is_console", false),
            ("game_pad", false),
            ("can_splitscreen", false),
            ("is_secondary_client", false),
            ("requires_xbl_signin_to_play", false),
            ("is_editor_mode_enabled", false),
        ];
        platform
            .iter()
            .chain(constant)
            .fold(Self::desktop(), |context, (name, value)| {
                context.with_flag(name, *value)
            })
    }

    /// The variables set so far, keyed without `$`.
    pub fn vars(&self) -> &BTreeMap<String, Value> {
        &self.vars
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
