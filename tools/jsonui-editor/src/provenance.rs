//! Which file and layer set each resolved property. A node's contributing
//! definition sites are its instance entries under each of its parent's sites,
//! then its `@base` chain; a property comes from the most-derived site that
//! writes it, from the latest layer and file that wrote it there. A `$var`
//! value also names where that variable was declared.

use json_ui::{Catalog, ControlRef, ResolvedControl};
use serde::Serialize;
use serde_json::Value;

use crate::diagnose::Location;
use crate::index::Index;
use crate::workspace::Workspace;

const MAX_CHAIN: usize = 64;
/// Properties the resolver adds rather than any file.
const ENGINE_KEYS: &[&str] = &[
    "anim_alpha",
    "anim_born",
    "anim_clock",
    "factory_scope",
    "factory_scope_key",
    "collection_index",
];

#[derive(Clone, Debug, Serialize)]
pub struct Property {
    pub key: String,
    pub value: Value,
    pub source: Source,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// Written in a file: the site (`ns.control/child`) and where.
    File {
        site: String,
        layer_name: String,
        location: Location,
        /// The `$var` the written value read, and where it was declared.
        #[serde(skip_serializing_if = "Option::is_none")]
        via: Option<Box<Via>>,
    },
    /// Baked from the data source by a binding.
    Binding,
    /// Added by the engine (animation chains, factory scope, indices).
    Engine,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct Via {
    pub variable: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared: Option<Location>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declared_site: Option<String>,
}

/// Everything the inspector shows for one control.
#[derive(Clone, Debug, Serialize)]
pub struct Inspection {
    pub name: String,
    #[serde(rename = "type")]
    pub control_type: Option<String>,
    pub base: Option<String>,
    pub sites: Vec<String>,
    pub definition: Option<Location>,
    pub properties: Vec<Property>,
}

pub struct Tracer<'a> {
    pub catalog: &'a Catalog,
    pub index: &'a Index,
    pub workspace: &'a Workspace,
}

impl Tracer<'_> {
    /// Sites of every node along `path` from `root`, root first; `reference`
    /// names the root's definition.
    pub fn sites_along<'r>(
        &self,
        reference: &str,
        root: &'r ResolvedControl,
        path: &[usize],
    ) -> Vec<(Vec<String>, &'r ResolvedControl)> {
        let mut out = Vec::new();
        let Some((namespace, name)) = reference.split_once('.') else {
            return out;
        };
        let mut sites = self.chain(&ControlRef::new(namespace, name));
        let mut node = root;
        out.push((sites.clone(), node));
        for &index in path {
            let Some(child) = node.children.get(index) else {
                break;
            };
            sites = self.child_sites(&sites, child);
            node = child;
            out.push((sites.clone(), node));
        }
        out
    }

    fn child_sites(&self, parent: &[String], child: &ResolvedControl) -> Vec<String> {
        let mut sites: Vec<String> = parent
            .iter()
            .map(|site| format!("{site}/{}", child.name))
            .filter(|site| self.index.has_site(site))
            .collect();
        if let Some(base) = &child.base {
            sites.extend(self.chain(base));
        }
        if sites.is_empty() {
            // A factory-created control: its top-level definition by name.
            for namespace in self.index.namespaces_of(&child.name) {
                let reference = ControlRef::new(namespace.clone(), child.name.clone());
                let base = self
                    .catalog
                    .lookup(namespace, &child.name)
                    .and_then(literal_base);
                if base == child.base {
                    sites.extend(self.chain(&reference));
                    break;
                }
            }
        }
        sites
    }

    /// `reference` and its literal `@base` chain, as sites.
    fn chain(&self, reference: &ControlRef) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = Some(reference.clone());
        while let Some(reference) = current.take() {
            let site = reference.to_string();
            if out.contains(&site) || out.len() >= MAX_CHAIN {
                break;
            }
            out.push(site);
            current = self
                .catalog
                .lookup(&reference.namespace, &reference.name)
                .and_then(literal_base);
        }
        out
    }

    /// Inspect the node at `path`: its sites, definition and property sources.
    pub fn inspect(
        &self,
        reference: &str,
        root: &ResolvedControl,
        path: &[usize],
    ) -> Option<Inspection> {
        let along = self.sites_along(reference, root, path);
        let (sites, node) = along.last()?;
        let definition = sites
            .iter()
            .find_map(|site| self.index.defs(site).first().copied())
            .map(|def| Location::at(self.workspace, def.layer, &def.path, def.start, def.end));
        let properties = node
            .properties
            .iter()
            .map(|(key, value)| Property {
                key: key.clone(),
                value: value.clone(),
                source: self.source(key, &along),
            })
            .collect();
        Some(Inspection {
            name: node.name.clone(),
            control_type: node.control_type.clone(),
            base: node.base.as_ref().map(ToString::to_string),
            sites: sites.clone(),
            definition,
            properties,
        })
    }

    fn source(&self, key: &str, along: &[(Vec<String>, &ResolvedControl)]) -> Source {
        if key.starts_with('#') {
            return Source::Binding;
        }
        if ENGINE_KEYS.contains(&key) {
            return Source::Engine;
        }
        let Some((sites, _)) = along.last() else {
            return Source::Unknown;
        };
        for site in sites {
            for def in self.index.defs(site) {
                let Some(span) = def.props.get(key) else {
                    continue;
                };
                let location = Location::at(self.workspace, def.layer, &def.path, span[0], span[1]);
                let via = self
                    .written_var(def.layer, &def.path, span)
                    .map(|variable| {
                        let declared = self.declaration(&variable, along);
                        Via {
                            variable,
                            declared: declared.as_ref().map(|(_, location)| location.clone()),
                            declared_site: declared.map(|(site, _)| site),
                        }
                    });
                return Source::File {
                    site: site.clone(),
                    layer_name: self.layer_name(def.layer),
                    location,
                    via: via.map(Box::new),
                };
            }
        }
        Source::Unknown
    }

    fn layer_name(&self, layer: usize) -> String {
        self.workspace
            .layer(layer)
            .map_or_else(String::new, |layer| layer.name.clone())
    }

    /// The `$var` a property's written value is, if it is one.
    fn written_var(&self, layer: usize, path: &str, span: &[usize; 4]) -> Option<String> {
        let text = self.workspace.layer(layer)?.text(path)?;
        let value = text.get(span[2]..span[3])?.trim();
        let inner = value.strip_prefix('"')?.strip_suffix('"')?;
        inner.starts_with('$').then(|| inner.to_owned())
    }

    /// Where `$var` was declared: the nearest enclosing site that sets it, else
    /// the topmost `_global_variables.json` defining it.
    fn declaration(
        &self,
        variable: &str,
        along: &[(Vec<String>, &ResolvedControl)],
    ) -> Option<(String, Location)> {
        let keys = [variable.to_owned(), format!("{variable}|default")];
        for (sites, _) in along.iter().rev() {
            for site in sites {
                for def in self.index.defs(site) {
                    if let Some(span) = keys.iter().find_map(|key| def.props.get(key)) {
                        let location =
                            Location::at(self.workspace, def.layer, &def.path, span[0], span[1]);
                        return Some((site.clone(), location));
                    }
                }
            }
        }
        let globals = "ui/_global_variables.json";
        for layer in (0..self.workspace.layers().len()).rev() {
            let Some(text) = self.workspace.layers()[layer].text(globals) else {
                continue;
            };
            let Ok(root) = crate::outline::parse(&text) else {
                continue;
            };
            if let Some(member) = root.members().iter().find(|m| m.key == variable) {
                let location = Location::at(
                    self.workspace,
                    layer,
                    globals,
                    member.key_start,
                    member.key_end,
                );
                return Some(("global".to_owned(), location));
            }
        }
        None
    }
}

fn literal_base(control: &json_ui::RawControl) -> Option<ControlRef> {
    let base = control
        .base
        .as_deref()
        .filter(|base| !base.starts_with('$'))?;
    Some(ControlRef::parse(base, &control.owner_ns))
}
