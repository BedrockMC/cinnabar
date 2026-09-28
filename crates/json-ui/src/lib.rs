//! Clean-room parser and resolver for vanilla Bedrock JSON-UI definitions.
//!
//! This tranche reads the on-disk `ui/*.json` (tolerant JSON5), applies `@base`
//! inheritance, substitutes `$var`/global references, evaluates `ignored` and
//! `variables[]` conditionals for the given screen context, and records factory
//! mappings. It emits a concrete, serializable control tree ([`ResolvedControl`])
//! and renders nothing. Size, offset and `view` expressions are deliberately left
//! symbolic for a later arithmetic/layout stage.

mod catalog;
mod env;
mod json5;
mod merge;
mod predicate;
mod resolve;
mod tree;

use std::collections::BTreeMap;

use serde_json::Value;

pub use catalog::{Catalog, LoadError, RawControl};
pub use env::Env;
pub use resolve::Resolver;
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
