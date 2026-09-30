//! Laid-out engine screens reused while their inputs are unchanged, so a static
//! menu only repaints each frame instead of resolving, binding and laying out.

use std::sync::{Arc, Mutex};

use json_ui::{Catalog, Context, DataSource, FormRender, ViewState};

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
    /// A translated probe string, so a language change relays out.
    pub(super) language: Option<Arc<str>>,
}

struct Entry {
    reference: String,
    catalog: Arc<Catalog>,
    context: Context,
    data: DataSource,
    view: ViewState,
    root: [f64; 2],
    px: f32,
    language: Option<Arc<str>>,
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

#[derive(Default)]
pub(super) struct ScreenCache(Mutex<Vec<Entry>>);

impl ScreenCache {
    /// The cached render for `key`, else `render()`'s, remembered in place of
    /// the oldest entry.
    pub(super) fn get_or_render(
        &self,
        key: ScreenKey<'_>,
        render: impl FnOnce() -> Option<FormRender>,
    ) -> Option<Arc<FormRender>> {
        let mut entries = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
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
            language: None,
        };
        let first = cache.get_or_render(key([400.0, 300.0]), render).unwrap();
        let again = cache
            .get_or_render(key([400.0, 300.0]), || panic!("rendered twice"))
            .unwrap();
        assert!(Arc::ptr_eq(&first, &again));
        let resized = cache.get_or_render(key([500.0, 300.0]), render).unwrap();
        assert!(!Arc::ptr_eq(&first, &resized));
    }
}
