//! Laid-out engine screens reused while their inputs are unchanged, so a static
//! menu only repaints each frame; hover, press and focus are gated in the
//! layout and only repaint. Resolved trees are reused across value changes,
//! which only re-bind and re-lay out, and a screen about to open (Settings from
//! the start screen) is laid out on a background thread ahead of time.

use std::{
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use json_ui::{Catalog, Context, DataSource, FormRender, ResolvedControl, ViewState};

/// Screens kept at once: a menu, its overlay and a dialog popup.
const SLOTS: usize = 4;

/// Everything a screen's layout depends on besides the catalog's contents.
pub(super) struct ScreenKey<'a> {
    pub(super) reference: &'a str,
    pub(super) catalog: &'a Arc<Catalog>,
    pub(super) context: &'a Context,
    pub(super) data: &'a DataSource,
    /// Only its scroll offsets key the layout.
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
            && self.view.scroll == key.view.scroll
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
    /// Shared with preparing threads.
    laid: Arc<Mutex<Vec<Entry>>>,
    resolved: Arc<Mutex<Vec<Resolved>>>,
    /// The screen a preparing thread is laying out; notified when it finishes.
    preparing: Arc<(Mutex<Option<&'static str>>, Condvar)>,
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
        let mut entries = lock(&self.laid);
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
            view: ViewState {
                scroll: key.view.scroll.clone(),
                ..ViewState::default()
            },
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
        self.await_preparation(&key);
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
            let measures = &mut json_ui::MeasureCache::default();
            Some(json_ui::render_bound_gated(
                bound, root, env, view, measures,
            ))
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
        let mut bindings = lock(&self.bindings);
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

    /// A cache already resolving the settings screen on a background thread,
    /// which its preparation or an early open then waits for.
    pub(super) fn resolving_settings(catalog: &Arc<Catalog>) -> Self {
        let (reference, context) = super::super::menu_screens::settings_target();
        let cache = Self::default();
        *lock(&cache.preparing.0) = Some(reference);
        let (resolved, preparing, catalog) = (
            Arc::clone(&cache.resolved),
            Arc::clone(&cache.preparing),
            Arc::clone(catalog),
        );
        let spawned = std::thread::Builder::new()
            .name("screen-resolve".to_owned())
            .spawn(move || {
                resolved_in(&resolved, reference, &catalog, &context, || {
                    json_ui::resolve(&catalog, reference, &context).control
                });
                *lock(&preparing.0) = None;
                preparing.1.notify_all();
            });
        if spawned.is_err() {
            *lock(&cache.preparing.0) = None;
        }
        cache
    }

    /// On a miss for the screen a thread is preparing, waits for it: finishing
    /// that is never slower than laying the screen out again here.
    fn await_preparation(&self, key: &ScreenKey<'_>) {
        let (flag, done) = &*self.preparing;
        let running = lock(flag);
        if *running == Some(key.reference)
            && !lock(&self.laid).iter().any(|entry| entry.matches(key))
        {
            let wait = Duration::from_secs(2);
            let _ = done.wait_timeout_while(running, wait, |running| running.is_some());
        }
    }

    /// Resolve, bind and lay out `screen` on a background thread so opening it
    /// later is a cache hit; one preparation runs at a time.
    pub(super) fn prepare(&self, screen: Prepared, engine: &super::FormEngine) {
        // Only a no-pack catalog lays out identically off the frame.
        if !Arc::ptr_eq(&engine.catalog, &engine.base) {
            return;
        }
        let key = ScreenKey {
            reference: screen.reference,
            catalog: &engine.catalog,
            context: &screen.context,
            data: &screen.data,
            view: &ViewState::default(),
            root: screen.root,
            px: screen.px,
            language: screen.language,
        };
        if lock(&self.laid).iter().any(|entry| entry.matches(&key)) {
            return;
        }
        let mut preparing = lock(&self.preparing.0);
        if preparing.is_some() {
            return;
        }
        *preparing = Some(screen.reference);
        drop(preparing);
        let detached = Detached {
            catalog: Arc::clone(&engine.catalog),
            assets: Arc::clone(&engine.assets),
            textures: engine.textures.detached(),
            laid: Arc::clone(&self.laid),
            resolved: Arc::clone(&self.resolved),
            preparing: Arc::clone(&self.preparing),
        };
        let spawned = std::thread::Builder::new()
            .name("screen-prepare".to_owned())
            .spawn(move || detached.lay_out(screen));
        if spawned.is_err() {
            *lock(&self.preparing.0) = None;
        }
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
        resolved_in(&self.resolved, reference, catalog, context, resolve)
    }
}

/// [`ScreenCache::resolved`] over a shared list, resolving outside its lock.
fn resolved_in(
    list: &Mutex<Vec<Resolved>>,
    reference: &str,
    catalog: &Arc<Catalog>,
    context: &Context,
    resolve: impl FnOnce() -> Option<ResolvedControl>,
) -> Option<Arc<ResolvedControl>> {
    let same = |entry: &Resolved| {
        entry.reference == reference
            && Arc::ptr_eq(&entry.catalog, catalog)
            && entry.context == *context
    };
    if let Some(entry) = lock(list).iter().find(|entry| same(entry)) {
        return Some(Arc::clone(&entry.root));
    }
    let root = Arc::new(resolve()?);
    let mut entries = lock(list);
    if !entries.iter().any(same) {
        if entries.len() >= SLOTS {
            entries.remove(0);
        }
        entries.push(Resolved {
            reference: reference.to_owned(),
            catalog: Arc::clone(catalog),
            context: context.clone(),
            root: Arc::clone(&root),
        });
    }
    Some(root)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A screen to lay out ahead of its first open, with what it measures text by.
pub(in super::super) struct Prepared {
    pub(in super::super) reference: &'static str,
    pub(in super::super) context: Context,
    pub(in super::super) data: DataSource,
    pub(in super::super) root: [f64; 2],
    pub(in super::super) px: f32,
    pub(in super::super) language: crate::ui_runtime::LanguageIdentity,
    pub(in super::super) font: Arc<assets::RuntimeFontCatalog>,
    pub(in super::super) metrics: super::super::super::TextMetrics,
    pub(in super::super) translator: crate::ui_runtime::Translator,
}

/// The engine state a background layout needs, owned by its thread.
struct Detached {
    catalog: Arc<Catalog>,
    assets: Arc<assets::RuntimeUiAssets>,
    textures: super::TextureSet,
    laid: Arc<Mutex<Vec<Entry>>>,
    resolved: Arc<Mutex<Vec<Resolved>>>,
    preparing: Arc<(Mutex<Option<&'static str>>, Condvar)>,
}

impl Detached {
    fn lay_out(self, screen: Prepared) {
        let (catalog, context) = (&self.catalog, &screen.context);
        let tree = resolved_in(&self.resolved, screen.reference, catalog, context, || {
            json_ui::resolve(catalog, screen.reference, context).control
        });
        if let Some(tree) = tree {
            let library = json_ui::CatalogLibrary { catalog, context };
            let bound = json_ui::bind(&tree, &screen.data, &library);
            let atlas = self.textures.lock();
            let textures = super::Textures {
                assets: &self.assets,
                set: &self.textures,
                atlas: &atlas,
                images: None,
            };
            let mut cache = ui::TextLayoutCache::new(
                super::super::super::TEXT_CACHE_ENTRIES,
                super::super::super::TEXT_CACHE_BYTES,
            );
            let layouts = std::cell::RefCell::new(&mut cache);
            let translate = |key: &str| screen.translator.lookup(key);
            let measure = super::Measure {
                layouts: &layouts,
                font: &screen.font,
                metrics: screen.metrics,
                px: screen.px,
                translate: &translate,
            };
            let env = json_ui::LayoutEnv {
                text: &measure,
                textures: &textures,
            };
            let view = ViewState::default();
            let measures = &mut json_ui::MeasureCache::default();
            let render = json_ui::render_bound_gated(bound, screen.root, &env, &view, measures);
            let mut entries = lock(&self.laid);
            if entries.len() >= SLOTS {
                entries.remove(0);
            }
            entries.push(Entry {
                reference: screen.reference.to_owned(),
                catalog: Arc::clone(catalog),
                context: screen.context.clone(),
                data: screen.data.clone(),
                view,
                root: screen.root,
                px: screen.px,
                language: screen.language,
                render: Arc::new(render),
            });
        }
        *lock(&self.preparing.0) = None;
        self.preparing.1.notify_all();
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
