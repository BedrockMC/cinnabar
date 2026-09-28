//! Render-controller compilation: per rig, the texture candidates, part visibility and colours
//! its controllers select each tick, as expressions in the shared Molang tables.
use std::{collections::BTreeMap, path::Path};

use assets::{
    AssetError, EntityAssetKind, EntityAssetSource, EntityAssetSymbol, EntityGeometry,
    EntityRenderCandidate, EntityRenderData, EntityRenderLayer, EntityRenderSlot,
    EntityRenderVisibility, EntityRigBinding, EntityRigGeometryBinding,
};
use serde_json::{Map, Value};

use super::{
    super::{SourcePayloads, molang::MolangCompiler},
    clip::{read_json, required_object},
};

const MAX_CANDIDATES_PER_SLOT: usize = 256;
const MAX_SLOTS_PER_LAYER: usize = 16;
const MAX_EXPANSION_DEPTH: usize = 8;

pub(super) struct RenderSources<'a> {
    pub root: &'a Path,
    pub payloads: &'a SourcePayloads,
    pub sources: &'a [EntityAssetSource],
    pub symbols: &'a [EntityAssetSymbol],
    pub geometries: &'a [EntityGeometry],
    pub rigs: &'a [EntityRigBinding],
    pub rig_geometries: &'a [EntityRigGeometryBinding],
}

/// One step of the path to a texture leaf.
#[derive(Clone)]
enum Step {
    /// A ternary condition and the branch taken.
    Branch(String, bool),
    /// Element `index` of an array of `len` entries selected by an index expression.
    Element(String, usize, usize),
}

fn condition_text(path: &[Step]) -> Option<String> {
    let parts: Vec<String> = path
        .iter()
        .map(|step| match step {
            Step::Branch(text, true) => format!("({text})"),
            Step::Branch(text, false) => format!("!({text})"),
            Step::Element(index, len, element) => {
                format!("(math.mod(math.max(math.floor(({index})), 0), {len}) == {element})")
            }
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" && "))
}

struct TextureScope<'a> {
    /// Lowercased alias to raster path stem.
    aliases: BTreeMap<String, String>,
    /// Lowercased `array.name` to member expressions.
    arrays: BTreeMap<String, Vec<String>>,
    sources: &'a [EntityAssetSource],
}

impl TextureScope<'_> {
    fn source_of(&self, stem: &str) -> Option<u32> {
        [".png", ".tga"].into_iter().find_map(|extension| {
            let path = format!("{stem}{extension}");
            self.sources
                .binary_search_by(|source| source.path.as_ref().cmp(path.as_str()))
                .ok()
                .map(|index| index as u32)
        })
    }

    /// Every texture the expression can select, with the conditions selecting each.
    fn expand(
        &self,
        expression: &str,
        path: &mut Vec<Step>,
        output: &mut Vec<(Vec<Step>, u32)>,
        depth: usize,
    ) -> Option<()> {
        if depth > MAX_EXPANSION_DEPTH || output.len() > MAX_CANDIDATES_PER_SLOT {
            return None;
        }
        let expression = strip_outer_parentheses(expression.trim());
        if let Some((condition, then_branch, else_branch)) = split_ternary(expression) {
            path.push(Step::Branch(condition.to_owned(), true));
            let then_result = self.expand(then_branch, path, output, depth + 1);
            path.pop();
            path.push(Step::Branch(condition.to_owned(), false));
            let else_result = self.expand(else_branch?, path, output, depth + 1);
            path.pop();
            return (then_result.is_some() && else_result.is_some()).then_some(());
        }
        let lower = expression.to_ascii_lowercase();
        if let Some(alias) = lower.strip_prefix("texture.") {
            if let Some(source) = self
                .aliases
                .get(alias)
                .and_then(|stem| self.source_of(stem))
            {
                output.push((path.clone(), source));
            }
            return Some(());
        }
        let (name, index) = split_index(expression)?;
        let members = self.arrays.get(&name.to_ascii_lowercase())?;
        for (element, member) in members.iter().enumerate() {
            path.push(Step::Element(index.to_owned(), members.len(), element));
            let result = self.expand(member, path, output, depth + 1);
            path.pop();
            result?;
        }
        Some(())
    }
}

fn strip_outer_parentheses(mut text: &str) -> &str {
    while text.starts_with('(') && text.ends_with(')') {
        let mut depth = 0i32;
        let mut encloses = true;
        for (offset, ch) in text.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 && offset + ch.len_utf8() < text.len() {
                encloses = false;
                break;
            }
        }
        if !encloses {
            break;
        }
        text = text[1..text.len() - 1].trim();
    }
    text
}

/// `(condition, then, else)` of a top-level `?:`; the else part is `None` for a bare `?`.
fn split_ternary(text: &str) -> Option<(&str, &str, Option<&str>)> {
    let mut depth = 0i32;
    let mut quoted = false;
    let mut question = None;
    let mut nested = 0i32;
    for (offset, ch) in text.char_indices() {
        if ch == '\'' {
            quoted = !quoted;
        }
        if quoted {
            continue;
        }
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '?' if depth == 0 => match question {
                None => question = Some(offset),
                Some(_) => nested += 1,
            },
            ':' if depth == 0 && question.is_some() => {
                if nested == 0 {
                    let question = question?;
                    return Some((
                        text[..question].trim(),
                        text[question + 1..offset].trim(),
                        Some(text[offset + 1..].trim()),
                    ));
                }
                nested -= 1;
            }
            _ => {}
        }
    }
    question.map(|question| (text[..question].trim(), text[question + 1..].trim(), None))
}

/// `name` and `index` of `name[index]`.
fn split_index(text: &str) -> Option<(&str, &str)> {
    let open = text.find('[')?;
    let inner = text.strip_suffix(']')?;
    Some((text[..open].trim(), inner[open + 1..].trim()))
}

fn expression_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(if *flag { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

fn lowercase_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(alias, target)| {
            Some((alias.to_ascii_lowercase(), target.as_str()?.to_owned()))
        })
        .collect()
}

/// Geometry identifiers a controller's `geometry` expression can select.
fn controller_geometries(
    definition: &Map<String, Value>,
    aliases: &BTreeMap<String, String>,
) -> Vec<String> {
    let Some(expression) = definition.get("geometry").and_then(Value::as_str) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    let lower = expression.to_ascii_lowercase();
    for token in lower.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')) {
        if let Some(alias) = token.strip_prefix("geometry.") {
            names.extend(aliases.get(alias).cloned());
        } else if let Some(array) = token.strip_prefix("array.") {
            let members = definition
                .get("arrays")
                .and_then(|arrays| arrays.get("geometries"))
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .find(|(name, _)| name.to_ascii_lowercase() == format!("array.{array}"))
                .and_then(|(_, members)| members.as_array());
            for member in members.into_iter().flatten().filter_map(Value::as_str) {
                if let Some(alias) = member.to_ascii_lowercase().strip_prefix("geometry.") {
                    names.extend(aliases.get(alias).cloned());
                }
            }
        }
    }
    names
}

fn compile_condition(molang: &mut MolangCompiler, text: &str) -> Option<u32> {
    molang.compile(text).ok()
}

fn compile_color(
    molang: &mut MolangCompiler,
    value: Option<&Value>,
    default_alpha: &str,
) -> Option<[u32; 4]> {
    let object = value?.as_object()?;
    let mut components = [0; 4];
    for (slot, (key, default)) in [
        ("r", "1.0"),
        ("g", "1.0"),
        ("b", "1.0"),
        ("a", default_alpha),
    ]
    .into_iter()
    .enumerate()
    {
        let text = object
            .get(key)
            .and_then(expression_text)
            .unwrap_or_else(|| default.to_owned());
        components[slot] = compile_condition(molang, &text)?;
    }
    Some(components)
}

pub(super) fn compile_render(
    input: RenderSources<'_>,
    molang: &mut MolangCompiler,
) -> Result<EntityRenderData, AssetError> {
    let RenderSources {
        root,
        payloads,
        sources,
        symbols,
        geometries,
        rigs,
        rig_geometries,
    } = input;
    let mut controllers = BTreeMap::<Box<str>, Option<Value>>::new();
    for source in sources
        .iter()
        .filter(|source| source.path.starts_with("render_controllers/"))
    {
        let value = read_json(root, payloads, source)?;
        for (name, definition) in required_object(&value, "render_controllers")? {
            controllers
                .entry(name.as_str().into())
                .and_modify(|existing| *existing = None)
                .or_insert_with(|| Some(definition.clone()));
        }
    }
    let mut entity_json = BTreeMap::<u32, Value>::new();
    let (mut layers, mut slots, mut candidates, mut visibility) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (rig_index, rig) in rigs.iter().enumerate() {
        let entity = &symbols[rig.entity_symbol as usize];
        debug_assert_eq!(entity.kind, EntityAssetKind::Entity);
        if !entity_json.contains_key(&entity.source_index) {
            let value = read_json(root, payloads, &sources[entity.source_index as usize])?;
            entity_json.insert(entity.source_index, value);
        }
        let Some(description) = entity_json[&entity.source_index]
            .get("minecraft:client_entity")
            .and_then(|value| value.get("description"))
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(rig_geometry) = rig_geometries.get(rig.first_geometry as usize) else {
            continue;
        };
        let default_geometry = geometries[rig_geometry.geometry as usize]
            .identifier
            .to_ascii_lowercase();
        let geometry_aliases = lowercase_map(description.get("geometry"));
        let scope_aliases = lowercase_map(description.get("textures"));
        let entries = description
            .get("render_controllers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for entry in entries {
            let (name, activation) = match entry {
                Value::String(name) => (name.as_str(), None),
                Value::Object(conditional) if conditional.len() == 1 => {
                    let (name, condition) = conditional.iter().next().expect("one entry");
                    (name.as_str(), expression_text(condition))
                }
                _ => continue,
            };
            let Some(Some(definition)) = controllers.get(name) else {
                continue;
            };
            let Some(definition) = definition.as_object() else {
                continue;
            };
            let selected = controller_geometries(definition, &geometry_aliases);
            if !selected.is_empty()
                && !selected
                    .iter()
                    .any(|identifier| identifier.to_ascii_lowercase() == default_geometry)
            {
                continue;
            }
            let condition = match activation {
                None => None,
                Some(text) => match compile_condition(molang, &text) {
                    Some(index) => Some(index),
                    None => continue,
                },
            };
            let arrays = definition
                .get("arrays")
                .and_then(|arrays| arrays.get("textures"))
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
                .map(|(name, members)| {
                    (
                        name.to_ascii_lowercase(),
                        members
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|member| member.as_str().map(str::to_owned))
                            .collect(),
                    )
                })
                .collect();
            let scope = TextureScope {
                aliases: scope_aliases.clone(),
                arrays,
                sources,
            };
            let first_slot = slots.len();
            for expression in definition
                .get("textures")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(MAX_SLOTS_PER_LAYER)
            {
                let mut leaves = Vec::new();
                if scope
                    .expand(expression, &mut Vec::new(), &mut leaves, 0)
                    .is_none()
                {
                    continue;
                }
                let first_candidate = candidates.len();
                for (path, source) in leaves {
                    let condition = match condition_text(&path) {
                        None => None,
                        Some(text) => match compile_condition(molang, &text) {
                            Some(index) => Some(index),
                            None => continue,
                        },
                    };
                    candidates.push(EntityRenderCandidate { condition, source });
                }
                let count = candidates.len() - first_candidate;
                if count == 0 {
                    continue;
                }
                slots.push(EntityRenderSlot {
                    first_candidate: first_candidate as u32,
                    candidate_count: count as u16,
                });
            }
            if slots.len() == first_slot {
                continue;
            }
            let first_visibility = visibility.len();
            for rule in definition
                .get("part_visibility")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
            {
                for (pattern, value) in rule {
                    let Some(index) =
                        expression_text(value).and_then(|text| compile_condition(molang, &text))
                    else {
                        continue;
                    };
                    visibility.push(EntityRenderVisibility {
                        pattern: pattern.to_ascii_lowercase().into(),
                        condition: index,
                    });
                }
            }
            layers.push(EntityRenderLayer {
                rig: rig_index as u32,
                condition,
                first_slot: first_slot as u32,
                slot_count: (slots.len() - first_slot) as u16,
                first_visibility: first_visibility as u32,
                visibility_count: (visibility.len() - first_visibility) as u16,
                color: compile_color(molang, definition.get("color"), "1.0"),
                overlay_color: compile_color(molang, definition.get("overlay_color"), "0.0"),
                on_fire_color: compile_color(molang, definition.get("on_fire_color"), "0.0"),
            });
        }
    }
    Ok(EntityRenderData {
        layers: layers.into_boxed_slice(),
        slots: slots.into_boxed_slice(),
        candidates: candidates.into_boxed_slice(),
        visibility: visibility.into_boxed_slice(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(sources: &[EntityAssetSource]) -> TextureScope<'_> {
        TextureScope {
            aliases: BTreeMap::from([
                ("a".into(), "textures/entity/a".into()),
                ("b".into(), "textures/entity/b".into()),
            ]),
            arrays: BTreeMap::from([(
                "array.skins".into(),
                vec!["Texture.a".into(), "Texture.b".into()],
            )]),
            sources,
        }
    }

    fn source(path: &str) -> EntityAssetSource {
        EntityAssetSource {
            path: path.into(),
            source_bytes: 1,
            source_sha256: [1; 32],
        }
    }

    #[test]
    fn ternaries_and_arrays_expand_into_conditioned_leaves() {
        let sources = [
            source("textures/entity/a.png"),
            source("textures/entity/b.png"),
        ];
        let scope = scope(&sources);
        let mut leaves = Vec::new();
        scope
            .expand(
                "query.is_baby ? Texture.a : Array.skins[query.variant]",
                &mut Vec::new(),
                &mut leaves,
                0,
            )
            .unwrap();
        let texts: Vec<_> = leaves
            .iter()
            .map(|(path, source)| (condition_text(path).unwrap(), *source))
            .collect();
        assert_eq!(texts.len(), 3);
        assert_eq!(texts[0], ("(query.is_baby)".into(), 0));
        assert!(texts[1].0.starts_with("!(query.is_baby) && (math.mod("));
        assert_eq!(texts[2].1, 1);
    }

    #[test]
    fn unsupported_atoms_reject_the_slot_and_plain_aliases_are_unconditional() {
        let sources = [source("textures/entity/a.png")];
        let scope = scope(&sources);
        let mut leaves = Vec::new();
        assert!(
            scope
                .expand("variable.foo", &mut Vec::new(), &mut leaves, 0)
                .is_none()
        );
        let mut leaves = Vec::new();
        scope
            .expand("Texture.a", &mut Vec::new(), &mut leaves, 0)
            .unwrap();
        assert_eq!(leaves.len(), 1);
        assert!(condition_text(&leaves[0].0).is_none());
    }

    #[test]
    fn ternary_split_respects_nesting_brackets_and_quotes() {
        assert_eq!(
            split_ternary("a ? b ? c : d : e"),
            Some(("a", "b ? c : d", Some("e")))
        );
        assert_eq!(
            split_ternary("query.x('m?n') ? Array.t[q ? 1 : 0] : d"),
            Some(("query.x('m?n')", "Array.t[q ? 1 : 0]", Some("d")))
        );
        assert_eq!(strip_outer_parentheses("(a ? b : c)"), "a ? b : c");
        assert_eq!(strip_outer_parentheses("(a) ? (b) : c"), "(a) ? (b) : c");
    }
}
