//! Catalogs built from in-memory `ui/*.json` bytes (the compiled carrier) and the
//! resource-pack overlay a joined server applies on top. An overlay control with
//! the same namespace and name as an existing one merges into it: each property
//! it names replaces the old value, a `controls` array replaces the children, and
//! `modifications` edits arrays in place (insert/remove/replace/move/swap by
//! control name for `controls`, by a `where` field match for other arrays). A
//! control the base lacks is simply added. Overlay `_global_variables.json`
//! values override the base's.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};

use crate::catalog::{Catalog, LoadError, RawControl, child_controls, parse_ui_defs, split_key};
use crate::json5;

const GLOBALS: &str = "ui/_global_variables.json";
const UI_DEFS: &str = "ui/_ui_defs.json";

impl Catalog {
    /// Load a base catalog from `(pack-relative path, bytes)` pairs, honouring the
    /// `ui/_ui_defs.json` load order. Both index files are required.
    pub fn from_files<'a>(
        files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> Result<Self, LoadError> {
        let files: BTreeMap<&str, &[u8]> = files.into_iter().collect();
        let text = |path: &str| -> Result<String, LoadError> {
            let bytes = files.get(path).ok_or_else(|| LoadError::Shape {
                path: Path::new(path).to_path_buf(),
            })?;
            Ok(String::from_utf8_lossy(bytes).into_owned())
        };
        let mut catalog = Catalog::default();
        catalog.load_globals_text(Path::new(GLOBALS), &text(GLOBALS)?)?;
        let entries = parse_ui_defs(Path::new(UI_DEFS), &text(UI_DEFS)?)?;
        catalog.list(entries.iter().cloned());
        for entry in entries {
            match files.get(entry.as_str()) {
                Some(bytes) => catalog.load_text(&entry, &String::from_utf8_lossy(bytes)),
                None => catalog.note(format!("{entry}: listed but absent")),
            }
        }
        Ok(catalog)
    }

    /// Overlay a resource pack's ui files (pack-relative paths). As the vanilla
    /// client does, only paths some `_ui_defs.json` lists load, in sorted order;
    /// malformed files are skipped and recorded, never fatal.
    pub fn apply_pack<'a>(&mut self, files: impl IntoIterator<Item = (&'a str, &'a [u8])>) {
        let files: BTreeMap<&str, &[u8]> = files
            .into_iter()
            .filter(|(path, _)| path.starts_with("ui/") && path.ends_with(".json"))
            .collect();
        if let Some(bytes) = files.get(GLOBALS)
            && let Err(error) =
                self.load_globals_text(Path::new(GLOBALS), &String::from_utf8_lossy(bytes))
        {
            self.note(format!("pack {GLOBALS}: {error}"));
        }
        if let Some(bytes) = files.get(UI_DEFS) {
            match parse_ui_defs(Path::new(UI_DEFS), &String::from_utf8_lossy(bytes)) {
                Ok(entries) => self.list(entries),
                Err(error) => self.note(format!("pack {UI_DEFS}: {error}")),
            }
        }
        for (path, bytes) in files {
            if path == GLOBALS || path == UI_DEFS {
                continue;
            }
            if self.lists(path) {
                self.merge_overlay_file(path, &String::from_utf8_lossy(bytes));
            } else {
                self.note(format!("pack {path}: not listed in any _ui_defs.json; skipped"));
            }
        }
    }

    /// The namespaces a pack's ui files define or extend (a file may omit the
    /// namespace of the vanilla file it overrides).
    pub fn overlay_namespaces<'a>(
        &self,
        files: impl IntoIterator<Item = (&'a str, &'a [u8])>,
    ) -> std::collections::BTreeSet<String> {
        files
            .into_iter()
            .filter(|(path, _)| {
                path.starts_with("ui/")
                    && path.ends_with(".json")
                    && *path != GLOBALS
                    && *path != UI_DEFS
            })
            .filter_map(
                |(path, bytes)| match json5::parse(&String::from_utf8_lossy(bytes)) {
                    Ok(Value::Object(object)) => match object.get("namespace") {
                        Some(Value::String(namespace)) => Some(namespace.clone()),
                        _ => self.file_namespace(path).map(str::to_owned),
                    },
                    _ => None,
                },
            )
            .collect()
    }

    pub(crate) fn merge_overlay_file(&mut self, entry: &str, text: &str) {
        let object = match json5::parse(text) {
            Ok(Value::Object(object)) => object,
            Ok(_) => return self.note(format!("pack {entry}: top level is not an object")),
            Err(error) => return self.note(format!("pack {entry}: parse error ({error})")),
        };
        // A file overriding a vanilla path may omit the namespace it extends.
        let namespace = match object.get("namespace") {
            Some(Value::String(namespace)) => namespace.clone(),
            _ => self
                .file_namespace(entry)
                .unwrap_or(crate::catalog::ROOT_NAMESPACE)
                .to_owned(),
        };
        for (key, body) in &object {
            if key == "namespace" {
                continue;
            }
            let mut diagnostics = Vec::new();
            // `parent/child` addresses a nested control by instance names.
            if let Some((top, rest)) = key.split_once('/') {
                let target = self
                    .lookup_mut(&namespace, top)
                    .and_then(|control| descendant(control, rest));
                match target {
                    Some(existing) => merge_into(existing, None, body, &mut diagnostics),
                    None => diagnostics.push(format!("{key}: nested control not found")),
                }
            } else {
                let (name, base) = split_key(key);
                match self.lookup_mut(&namespace, &name) {
                    Some(existing) => merge_into(existing, base, body, &mut diagnostics),
                    None => {
                        let control =
                            RawControl::from_entry(&namespace, key, body, &mut diagnostics);
                        self.insert(control);
                    }
                }
            }
            for message in diagnostics {
                self.note(format!("pack {entry}: {message}"));
            }
        }
    }
}

/// The control at a `/`-joined instance-name path below `control`.
fn descendant<'a>(control: &'a mut RawControl, path: &str) -> Option<&'a mut RawControl> {
    path.split('/').try_fold(control, |node, name| {
        node.children.iter_mut().find(|child| child.name == name)
    })
}

fn merge_into(
    existing: &mut RawControl,
    base: Option<String>,
    body: &Value,
    diagnostics: &mut Vec<String>,
) {
    if base.is_some() {
        existing.base = base;
    }
    let body = match body {
        Value::Object(body) => body,
        Value::Array(items) if items.is_empty() => return,
        _ => {
            diagnostics.push(format!("{}: control body is not an object", existing.name));
            return;
        }
    };
    for (property, value) in body {
        match property.as_str() {
            "controls" if value.is_string() => {
                existing.children.clear();
                existing.has_controls = true;
                existing.props.insert(property.clone(), value.clone());
            }
            "controls" => {
                existing.props.remove("controls");
                existing.has_controls = true;
                existing.children = child_controls(&existing.owner_ns, value, diagnostics);
            }
            "modifications" => apply_modifications(existing, value, diagnostics),
            _ => {
                existing.props.insert(property.clone(), value.clone());
            }
        }
    }
}

fn apply_modifications(control: &mut RawControl, value: &Value, diagnostics: &mut Vec<String>) {
    let Value::Array(items) = value else {
        diagnostics.push(format!("{}: `modifications` is not an array", control.name));
        return;
    };
    for item in items {
        let Some(item) = item.as_object() else {
            continue;
        };
        let array = item
            .get("array_name")
            .and_then(Value::as_str)
            .unwrap_or("controls");
        let operation = item.get("operation").and_then(Value::as_str).unwrap_or("");
        let applied = if array == "controls" {
            modify_controls(control, operation, item, diagnostics)
        } else {
            let entry = control
                .props
                .entry(array.to_owned())
                .or_insert_with(|| Value::Array(Vec::new()));
            match entry {
                Value::Array(values) => modify_values(values, operation, item),
                _ => false,
            }
        };
        if !applied {
            diagnostics.push(format!(
                "{}: modification `{operation}` on `{array}` matched nothing",
                control.name
            ));
        }
    }
}

fn modify_controls(
    control: &mut RawControl,
    operation: &str,
    item: &Map<String, Value>,
    diagnostics: &mut Vec<String>,
) -> bool {
    let owner = control.owner_ns.clone();
    let incoming = |diagnostics: &mut Vec<String>| {
        item.get("value")
            .map(|value| {
                let value = match value {
                    Value::Array(_) => value.clone(),
                    other => Value::Array(vec![other.clone()]),
                };
                child_controls(&owner, &value, diagnostics)
            })
            .unwrap_or_default()
    };
    let name = item.get("control_name").and_then(Value::as_str);
    let position = |children: &[RawControl], name: Option<&str>| {
        name.and_then(|name| children.iter().position(|child| child.name == name))
    };
    let children = &mut control.children;
    match operation {
        "insert_back" => children.extend(incoming(diagnostics)),
        "insert_front" => {
            let mut added = incoming(diagnostics);
            added.append(children);
            *children = added;
        }
        "insert_after" | "insert_before" => {
            let Some(at) = position(children.as_slice(), name) else {
                return false;
            };
            let at = if operation == "insert_after" {
                at + 1
            } else {
                at
            };
            let tail = children.split_off(at);
            children.extend(incoming(diagnostics));
            children.extend(tail);
        }
        "remove" => {
            let Some(at) = position(children.as_slice(), name) else {
                return false;
            };
            children.remove(at);
        }
        "replace" => {
            let Some(at) = position(children.as_slice(), name) else {
                return false;
            };
            let tail = children.split_off(at + 1);
            children.pop();
            children.extend(incoming(diagnostics));
            children.extend(tail);
        }
        "move_front" | "move_back" | "move_after" | "move_before" => {
            let Some(at) = position(children.as_slice(), name) else {
                return false;
            };
            let moved = children.remove(at);
            let target = item.get("target_control").and_then(Value::as_str);
            let index = match operation {
                "move_front" => 0,
                "move_back" => children.len(),
                _ => match position(children.as_slice(), target) {
                    Some(target) if operation == "move_after" => target + 1,
                    Some(target) => target,
                    None => {
                        children.insert(at, moved);
                        return false;
                    }
                },
            };
            children.insert(index, moved);
        }
        "swap" => {
            let target = item.get("target_control").and_then(Value::as_str);
            let (Some(a), Some(b)) = (
                position(children.as_slice(), name),
                position(children.as_slice(), target),
            ) else {
                return false;
            };
            children.swap(a, b);
        }
        _ => return false,
    }
    true
}

/// Edit a non-`controls` array, matching entries whose fields equal `where`.
fn modify_values(values: &mut Vec<Value>, operation: &str, item: &Map<String, Value>) -> bool {
    let matches = |value: &Value, key: &str| match (item.get(key), value) {
        (Some(Value::Object(pattern)), Value::Object(entry)) => pattern
            .iter()
            .all(|(field, expected)| entry.get(field) == Some(expected)),
        _ => false,
    };
    let incoming = || match item.get("value") {
        Some(Value::Array(added)) => added.clone(),
        Some(other) => vec![other.clone()],
        None => Vec::new(),
    };
    let found = |values: &[Value], key: &str| values.iter().position(|value| matches(value, key));
    match operation {
        "insert_back" => values.extend(incoming()),
        "insert_front" => {
            let mut added = incoming();
            added.append(values);
            *values = added;
        }
        "insert_after" | "insert_before" => {
            let Some(at) = found(values.as_slice(), "where") else {
                return false;
            };
            let at = if operation == "insert_after" {
                at + 1
            } else {
                at
            };
            let tail = values.split_off(at);
            values.extend(incoming());
            values.extend(tail);
        }
        "remove" => {
            let before = values.len();
            values.retain(|value| !matches(value, "where"));
            return values.len() != before;
        }
        "replace" => {
            let Some(at) = found(values.as_slice(), "where") else {
                return false;
            };
            let tail = values.split_off(at + 1);
            values.pop();
            values.extend(incoming());
            values.extend(tail);
        }
        "move_front" | "move_back" | "move_after" | "move_before" => {
            let Some(at) = found(values.as_slice(), "where") else {
                return false;
            };
            let moved = values.remove(at);
            let index = match operation {
                "move_front" => 0,
                "move_back" => values.len(),
                _ => match found(values.as_slice(), "target") {
                    Some(target) if operation == "move_after" => target + 1,
                    Some(target) => target,
                    None => {
                        values.insert(at, moved);
                        return false;
                    }
                },
            };
            values.insert(index, moved);
        }
        "swap" => {
            let (Some(a), Some(b)) = (
                found(values.as_slice(), "where"),
                found(values.as_slice(), "target"),
            ) else {
                return false;
            };
            values.swap(a, b);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Catalog {
        let globals = br##"{ "$color": "white" }"##;
        let defs = br##"{ "ui_defs": ["ui/screen.json"] }"##;
        let screen = br##"{
            "namespace": "screen",
            "panel": {
                "type": "panel",
                "size": [10, 10],
                "bindings": [ { "binding_name": "#a" }, { "binding_name": "#b" } ],
                "controls": [ { "first": { "type": "image" } }, { "second": { "type": "label" } } ]
            }
        }"##;
        Catalog::from_files([
            ("ui/_global_variables.json", globals.as_slice()),
            ("ui/_ui_defs.json", defs.as_slice()),
            ("ui/screen.json", screen.as_slice()),
        ])
        .expect("base loads")
    }

    fn names(catalog: &Catalog) -> Vec<String> {
        catalog
            .lookup("screen", "panel")
            .unwrap()
            .children
            .iter()
            .map(|child| child.name.clone())
            .collect()
    }

    #[test]
    fn overlay_properties_replace_and_keep_the_rest() {
        let mut catalog = base();
        let pack = br##"{ "namespace": "screen", "panel": { "size": [20, 5] } }"##;
        catalog.apply_pack([("ui/screen.json", pack.as_slice())]);
        let panel = catalog.lookup("screen", "panel").unwrap();
        assert_eq!(panel.props.get("size"), Some(&serde_json::json!([20, 5])));
        assert_eq!(panel.props.get("type"), Some(&serde_json::json!("panel")));
        assert_eq!(names(&catalog), ["first", "second"]);
    }

    #[test]
    fn control_modifications_insert_remove_and_move_by_name() {
        let mut catalog = base();
        let pack = br##"{ "namespace": "screen", "panel": { "modifications": [
            { "array_name": "controls", "operation": "insert_after", "control_name": "first",
              "value": [ { "middle": { "type": "panel" } } ] },
            { "array_name": "controls", "operation": "remove", "control_name": "second" },
            { "array_name": "controls", "operation": "move_front", "control_name": "middle" }
        ] } }"##;
        catalog.apply_pack([("ui/screen.json", pack.as_slice())]);
        assert_eq!(names(&catalog), ["middle", "first"]);
    }

    #[test]
    fn array_modifications_match_entries_by_where() {
        let mut catalog = base();
        let pack = br##"{ "namespace": "screen", "panel": { "modifications": [
            { "array_name": "bindings", "operation": "remove", "where": { "binding_name": "#a" } },
            { "array_name": "bindings", "operation": "insert_front", "value": { "binding_name": "#z" } }
        ] } }"##;
        catalog.apply_pack([("ui/screen.json", pack.as_slice())]);
        let bindings = catalog.lookup("screen", "panel").unwrap().props["bindings"].clone();
        assert_eq!(
            bindings,
            serde_json::json!([{ "binding_name": "#z" }, { "binding_name": "#b" }])
        );
    }

    #[test]
    fn overlay_globals_override_and_new_controls_are_added() {
        let mut catalog = base();
        catalog.apply_pack([
            (
                "ui/_global_variables.json",
                br##"{ "$color": "red" }"##.as_slice(),
            ),
            (
                "ui/_ui_defs.json",
                br##"{ "ui_defs": ["ui/new.json"] }"##.as_slice(),
            ),
            (
                "ui/new.json",
                br##"{ "namespace": "added", "thing": { "type": "panel" } }"##.as_slice(),
            ),
        ]);
        assert_eq!(catalog.global("color"), Some(&serde_json::json!("red")));
        assert!(catalog.lookup("added", "thing").is_some());
    }
}
