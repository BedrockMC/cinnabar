//! Raw, pre-resolution model: every `ui/*.json` control kept as authored, keyed
//! by `namespace` then local name, with `$global` variables loaded alongside.
//! Property maps are preserved verbatim (minus `controls`, which becomes the child
//! list) so later stages consume unknown keys unchanged.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::json5;

/// A control exactly as authored, with its `@base` recorded from the key.
#[derive(Clone, Debug)]
pub struct RawControl {
    /// Namespace of the file this control was authored in; qualifies bare refs.
    pub owner_ns: String,
    /// Local instance name (the part before `@`).
    pub name: String,
    /// The raw `@base` reference (after `@`), possibly a bare name or a `$var`.
    pub base: Option<String>,
    /// All properties except `controls`.
    pub props: Map<String, Value>,
    /// Nested controls, in document order.
    pub children: Vec<RawControl>,
}

impl RawControl {
    pub(crate) fn from_entry(
        owner_ns: &str,
        key: &str,
        value: &Value,
        diagnostics: &mut Vec<String>,
    ) -> Self {
        let (name, base) = split_key(key);
        let mut props = Map::new();
        let mut children = Vec::new();
        match value {
            Value::Object(object) => {
                for (property, item) in object {
                    match (property.as_str(), item) {
                        // A `$var` child list resolves against the scope later.
                        ("controls", Value::String(_)) => {
                            props.insert(property.clone(), item.clone());
                        }
                        ("controls", _) => children = child_controls(owner_ns, item, diagnostics),
                        _ => {
                            props.insert(property.clone(), item.clone());
                        }
                    }
                }
            }
            // Packs write an empty body as `[]`.
            Value::Array(items) if items.is_empty() => {}
            _ => diagnostics.push(format!("{owner_ns}.{name}: control body is not an object")),
        }
        Self {
            owner_ns: owner_ns.to_owned(),
            name,
            base,
            props,
            children,
        }
    }
}

pub(crate) fn child_controls(
    owner_ns: &str,
    value: &Value,
    diagnostics: &mut Vec<String>,
) -> Vec<RawControl> {
    let Value::Array(entries) = value else {
        diagnostics.push(format!("{owner_ns}: `controls` is not an array"));
        return Vec::new();
    };
    let mut children = Vec::new();
    for entry in entries {
        let Value::Object(object) = entry else {
            diagnostics.push(format!("{owner_ns}: `controls` entry is not an object"));
            continue;
        };
        // Each entry is a single-key object `{ "name@base": { .. } }`.
        for (key, body) in object {
            children.push(RawControl::from_entry(owner_ns, key, body, diagnostics));
        }
    }
    children
}

pub(crate) fn split_key(key: &str) -> (String, Option<String>) {
    match key.split_once('@') {
        Some((name, base)) => (name.to_owned(), Some(base.to_owned())),
        None => (key.to_owned(), None),
    }
}

/// The whole pack: variable globals plus every control keyed by namespace/name.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    globals: BTreeMap<String, Value>,
    defs: BTreeMap<String, BTreeMap<String, RawControl>>,
    /// Each loaded file's namespace, which a pack file at that path may omit.
    file_namespaces: BTreeMap<String, String>,
    diagnostics: Vec<String>,
}

impl Catalog {
    /// Load `_ui_defs.json`, `_global_variables.json`, and every listed `ui/*.json`
    /// in declared load order. `ui_dir` is the pack's `ui/` directory. Missing or
    /// malformed individual files are skipped and recorded; the two index files
    /// are required.
    pub fn load_dir(ui_dir: &Path) -> Result<Self, LoadError> {
        let pack_root = ui_dir.parent().unwrap_or(ui_dir);
        let mut catalog = Catalog::default();
        catalog.load_globals(&ui_dir.join("_global_variables.json"))?;
        let order = read_ui_defs(&ui_dir.join("_ui_defs.json"))?;
        for entry in order {
            let path = pack_root.join(&entry);
            catalog.load_file(&entry, &path);
        }
        Ok(catalog)
    }

    fn load_globals(&mut self, path: &Path) -> Result<(), LoadError> {
        let text = read(path)?;
        self.load_globals_text(path, &text)
    }

    pub(crate) fn load_globals_text(&mut self, path: &Path, text: &str) -> Result<(), LoadError> {
        let value = json5::parse(text).map_err(|source| LoadError::Parse {
            path: path.to_path_buf(),
            message: source.to_string(),
        })?;
        let Value::Object(object) = value else {
            return Err(LoadError::Shape {
                path: path.to_path_buf(),
            });
        };
        for (key, item) in object {
            if let Some(name) = key.strip_prefix('$') {
                self.globals.insert(name.to_owned(), item);
            }
        }
        Ok(())
    }

    fn load_file(&mut self, entry: &str, path: &Path) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => {
                self.diagnostics
                    .push(format!("{entry}: unreadable ({error})"));
                return;
            }
        };
        self.load_text(entry, &text);
    }

    /// Layers one pack `ui/*.json` file over the catalog with pack merge
    /// semantics. Bad files are recorded in diagnostics and skipped.
    pub fn overlay_text(&mut self, entry: &str, text: &str) {
        self.merge_overlay_file(entry, text);
    }

    /// Layers a pack's `_global_variables.json`; its variables replace earlier ones.
    pub fn overlay_globals_text(&mut self, text: &str) {
        match json5::parse(text) {
            Ok(Value::Object(object)) => {
                for (key, item) in object {
                    if let Some(name) = key.strip_prefix('$') {
                        self.globals.insert(name.to_owned(), item);
                    }
                }
            }
            _ => self
                .diagnostics
                .push("_global_variables.json: overlay is not an object".to_owned()),
        }
    }

    /// Add every control of one `ui/*.json` document; a redefinition replaces.
    pub(crate) fn load_text(&mut self, entry: &str, text: &str) {
        let value = match json5::parse(text) {
            Ok(value) => value,
            Err(error) => {
                self.diagnostics
                    .push(format!("{entry}: parse error ({error})"));
                return;
            }
        };
        let Value::Object(object) = value else {
            self.diagnostics
                .push(format!("{entry}: top level is not an object"));
            return;
        };
        let Some(Value::String(namespace)) = object.get("namespace") else {
            self.diagnostics
                .push(format!("{entry}: missing string `namespace`"));
            return;
        };
        let namespace = namespace.clone();
        self.file_namespaces
            .insert(entry.to_owned(), namespace.clone());
        for (key, body) in &object {
            if key == "namespace" {
                continue;
            }
            let control = RawControl::from_entry(&namespace, key, body, &mut self.diagnostics);
            let table = self.defs.entry(namespace.clone()).or_default();
            if let Some(existing) = table.insert(control.name.clone(), control) {
                self.diagnostics.push(format!(
                    "{namespace}.{}: redefined; last wins",
                    existing.name
                ));
            }
        }
    }

    pub fn lookup(&self, namespace: &str, name: &str) -> Option<&RawControl> {
        self.defs.get(namespace)?.get(name)
    }

    pub(crate) fn lookup_mut(&mut self, namespace: &str, name: &str) -> Option<&mut RawControl> {
        self.defs.get_mut(namespace)?.get_mut(name)
    }

    pub(crate) fn insert(&mut self, control: RawControl) {
        self.defs
            .entry(control.owner_ns.clone())
            .or_default()
            .insert(control.name.clone(), control);
    }

    pub(crate) fn file_namespace(&self, entry: &str) -> Option<&str> {
        self.file_namespaces.get(entry).map(String::as_str)
    }

    pub(crate) fn note(&mut self, message: String) {
        self.diagnostics.push(message);
    }

    pub fn global(&self, name: &str) -> Option<&Value> {
        self.globals.get(name)
    }

    pub fn globals(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.globals.iter()
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    pub fn namespace_count(&self) -> usize {
        self.defs.len()
    }
}

fn read_ui_defs(path: &Path) -> Result<Vec<String>, LoadError> {
    let text = read(path)?;
    parse_ui_defs(path, &text)
}

pub(crate) fn parse_ui_defs(path: &Path, text: &str) -> Result<Vec<String>, LoadError> {
    let value = json5::parse(text).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        message: source.to_string(),
    })?;
    let entries = value
        .get("ui_defs")
        .and_then(Value::as_array)
        .ok_or_else(|| LoadError::Shape {
            path: path.to_path_buf(),
        })?;
    Ok(entries
        .iter()
        .filter_map(|item| item.as_str().map(str::to_owned))
        .collect())
}

fn read(path: &Path) -> Result<String, LoadError> {
    std::fs::read_to_string(path).map_err(|source| LoadError::Read {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("unexpected shape in {path}")]
    Shape { path: PathBuf },
}

#[cfg(test)]
mod overlay_tests {
    use super::Catalog;

    // A pack file redefining a control replaces the vanilla one and adds new ones.
    #[test]
    fn overlay_text_replaces_and_adds_controls() {
        let mut catalog = Catalog::default();
        catalog.overlay_text("ui/a.json", r#"{"namespace":"n","box":{"size":[1,1]}}"#);
        catalog.overlay_text(
            "ui/b.json",
            r#"{"namespace":"n","box":{"size":[2,2]},"extra":{}}"#,
        );
        catalog.overlay_globals_text(r#"{"$g": 3}"#);
        assert_eq!(catalog.lookup("n", "box").unwrap().props["size"][0], 2);
        assert!(catalog.lookup("n", "extra").is_some());
        assert_eq!(
            catalog.global("g").and_then(|value| value.as_i64()),
            Some(3)
        );
    }
}
