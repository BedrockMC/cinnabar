//! Laid-out engine screens reused while their inputs are unchanged, so a static
//! menu only repaints each frame, and resolved trees reused across hover and
//! value changes, which only re-bind and re-lay out.

use std::sync::{Arc, Mutex};

use json_ui::{Catalog, Context, DataSource, FormRender, ResolvedControl, ViewState};

/// Screens kept at once: a menu, its overlay and a dialog popup.
const SLOTS: usize = 4;

/// Everything a screen's layout depends on besides the catalog's contents.
pub(super) struct ScreenKey<'a> {
    pub(super) reference: &'a str,
    pub(super) catalog: &'a Arc<Catalog>,
    pub(super) context: &'a Context,
    pub(super) data: &'a DataSource,
    pub(super) view: &'a ViewState,
    pub(super) root: [f64; 2],
    pub(super) px: f32,
    /// The active language tables, so a language change relays out.
    pub(super) language: crate::ui_runtime::LanguageIdentity,
}

struct Entry {
    reference: String,
    catalog: Arc<Catalog>,
    context: Context,
    data: DataSource,
    view: ViewState,
    root: [f64; 2],
    px: f32,
    language: crate::ui_runtime::LanguageIdentity,
    render: Arc<FormRender>,
}

impl Entry {
    fn matches(&self, key: &ScreenKey<'_>) -> bool {
        self.reference == key.reference
            && Arc::ptr_eq(&self.catalog, key.catalog)
            && self.root == key.root
            && self.px == key.px
            && self.language == key.language
            && self.view == *key.view
            && self.context == *key.context
            && self.data == *key.data
    }
}

/// A screen's resolved tree, which depends only on the catalog and context.
struct Resolved {
    reference: String,
    catalog: Arc<Catalog>,
    context: Context,
    root: Arc<ResolvedControl>,
}

#[derive(Default)]
pub(super) struct ScreenCache {
    laid: Mutex<Vec<Entry>>,
    /// Shared with prewarm threads.
    resolved: Arc<Mutex<Vec<Resolved>>>,
    /// Screens a prewarm thread was started for.
    warming: Mutex<Vec<String>>,
    /// Each screen's live bindings across data refreshes.
    bindings: Mutex<Vec<Bound>>,
}

/// A screen's binding state, which lives as long as its resolved tree.
struct Bound {
    reference: String,
    catalog: Arc<Catalog>,
    context: Context,
    state: json_ui::BindState,
}

impl ScreenCache {
    /// The cached render for `key`, else `render()`'s, remembered in place of
    /// the oldest entry.
    pub(super) fn get_or_render(
        &self,
        key: ScreenKey<'_>,
        render: impl FnOnce() -> Option<FormRender>,
    ) -> Option<Arc<FormRender>> {
        let mut entries = self
            .laid
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(index) = entries.iter().position(|entry| entry.matches(&key)) {
            let entry = entries.remove(index);
            let render = Arc::clone(&entry.render);
            entries.push(entry);
            return Some(render);
        }
        let rendered = Arc::new(render()?);
        if entries.len() >= SLOTS {
            entries.remove(0);
        }
        entries.push(Entry {
            reference: key.reference.to_owned(),
            catalog: Arc::clone(key.catalog),
            context: key.context.clone(),
            data: key.data.clone(),
            view: key.view.clone(),
            root: key.root,
            px: key.px,
            language: key.language,
            render: Arc::clone(&rendered),
        });
        Some(rendered)
    }

    /// An allow-listed screen's render for `key`: cached, else bound and laid
    /// out over its cached resolved tree.
    pub(super) fn render(
        &self,
        key: ScreenKey<'_>,
        env: &json_ui::LayoutEnv,
    ) -> Option<Arc<FormRender>> {
        let (reference, catalog, context, data, view, root) = (
            key.reference,
            key.catalog,
            key.context,
            key.data,
            key.view,
            key.root,
        );
        self.get_or_render(key, || {
            if !json_ui::is_engine_screen(reference) {
                return None;
            }
            let tree = self.resolved(reference, catalog, context, || {
                json_ui::resolve(catalog, reference, context).control
            })?;
            let library = json_ui::CatalogLibrary { catalog, context };
            let bound = self.with_binding(reference, catalog, context, |state| {
                json_ui::bind_stateful(&tree, data, &library, state).0
            });
            Some(json_ui::render_bound(bound, root, env, view))
        })
    }

    /// Run `bind` over `reference`'s binding state, created on first use.
    fn with_binding<T>(
        &self,
        reference: &str,
        catalog: &Arc<Catalog>,
        context: &Context,
        bind: impl FnOnce(&mut json_ui::BindState) -> T,
    ) -> T {
        let mut bindings = self
            .bindings
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let index = match bindings.iter().position(|bound| {
            bound.reference == reference
                && Arc::ptr_eq(&bound.catalog, catalog)
                && bound.context == *context
        }) {
            Some(index) => index,
            None => {
                if bindings.len() >= SLOTS {
                    bindings.remove(0);
                }
                bindings.push(Bound {
                    reference: reference.to_owned(),
                    catalog: Arc::clone(catalog),
                    context: context.clone(),
                    state: json_ui::BindState::new(),
                });
                bindings.len() - 1
            }
        };
        bind(&mut bindings[index].state)
    }

    /// Resolve `reference` under `context` on a background thread, once, so
    /// its first open does not stall a frame.
    pub(super) fn prewarm(
        &self,
        reference: &'static str,
        catalog: &Arc<Catalog>,
        context: Context,
    ) {
        let mut warming = self
            .warming
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if warming.iter().any(|started| started == reference) {
            return;
        }
        warming.push(reference.to_owned());
        let (resolved, catalog) = (Arc::clone(&self.resolved), Arc::clone(catalog));
        std::thread::spawn(move || {
            let Some(root) = json_ui::resolve(&catalog, reference, &context).control else {
                return;
            };
            let mut entries = resolved.lock().unwrap_or_else(|poison| poison.into_inner());
            let present = entries.iter().any(|entry| {
                entry.reference == reference
                    && Arc::ptr_eq(&entry.catalog, &catalog)
                    && entry.context == context
            });
            if !present {
                if entries.len() >= SLOTS {
                    entries.remove(0);
                }
                entries.push(Resolved {
                    reference: reference.to_owned(),
                    catalog,
                    context,
                    root: Arc::new(root),
                });
            }
        });
    }

    /// The resolved tree of `reference` under `context`, else `resolve()`'s;
    /// resolving the settings screen alone takes hundreds of milliseconds.
    pub(super) fn resolved(
        &self,
        reference: &str,
        catalog: &Arc<Catalog>,
        context: &Context,
        resolve: impl FnOnce() -> Option<ResolvedControl>,
    ) -> Option<Arc<ResolvedControl>> {
        let mut entries = self
            .resolved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(entry) = entries.iter().find(|entry| {
            entry.reference == reference
                && Arc::ptr_eq(&entry.catalog, catalog)
                && entry.context == *context
        }) {
            return Some(Arc::clone(&entry.root));
        }
        let root = Arc::new(resolve()?);
        if entries.len() >= SLOTS {
            entries.remove(0);
        }
        entries.push(Resolved {
            reference: reference.to_owned(),
            catalog: Arc::clone(catalog),
            context: context.clone(),
            root: Arc::clone(&root),
        });
        Some(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render() -> Option<FormRender> {
        Some(FormRender {
            bound: json_ui::ResolvedControl {
                name: "root".to_owned(),
                control_type: None,
                base: None,
                unresolved_base: None,
                properties: Default::default(),
                children: Vec::new(),
                factory: None,
            },
            nodes: Vec::new(),
            hits: Vec::new(),
            report: Default::default(),
            cancel_target: None,
            root_panel: None,
        })
    }

    // A second frame with the same inputs reuses the render; any change redoes it.
    #[test]
    fn unchanged_inputs_reuse_the_render() {
        let cache = ScreenCache::default();
        let catalog = Arc::new(Catalog::default());
        let (context, data, view) = (Context::desktop(), DataSource::new(), ViewState::default());
        let key = |root: [f64; 2]| ScreenKey {
            reference: "start.start_screen",
            catalog: &catalog,
            context: &context,
            data: &data,
            view: &view,
            root,
            px: 2.0,
            language: Default::default(),
        };
        let first = cache.get_or_render(key([400.0, 300.0]), render).unwrap();
        let again = cache
            .get_or_render(key([400.0, 300.0]), || panic!("rendered twice"))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let resized = cache.get_or_render(key([500.0, 300.0]), render).unwrap();
        assert!(!Arc::ptr_eq(&first, &resized));
        let tree = || render().map(|render| render.bound);
        let once = cache
            .resolved("start.start_screen", &catalog, &context, tree)
            .unwrap();
        let again = cache
            .resolved("start.start_screen", &catalog, &context, || {
                panic!("resolved twice")
            })
            .unwrap();
        assert!(Arc::ptr_eq(&once, &again));
    }
}
