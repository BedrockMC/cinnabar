//! Bounded authored activation, distinct from the alias lookup dictionary.
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Default)]
pub(crate) struct AuthoredRoots {
    pub clips: BTreeSet<String>,
    pub controllers: BTreeSet<String>,
}
impl AuthoredRoots {
    pub fn len(&self) -> usize {
        self.clips.len() + self.controllers.len()
    }
}

// Unsupported compatibility sources keep their existing parent behavior; they
// cannot qualify for neutral actor admission through this helper.
pub(crate) fn authored_roots(value: &Value) -> Option<AuthoredRoots> {
    let description = value.get("minecraft:client_entity")?.get("description")?;
    let aliases = description
        .get("animations")
        .map(Value::as_object)
        .transpose_option()?;
    let mut roots = AuthoredRoots::default();
    match value.get("format_version")?.as_str()? {
        "1.8.0" => {
            if description
                .get("scripts")
                .is_some_and(|scripts| scripts.as_object().is_none_or(|v| !v.is_empty()))
            {
                return None;
            }
            if aliases.is_some_and(|v| {
                v.values()
                    .any(|target| target.as_str().is_none_or(|s| !s.starts_with("animation.")))
            }) {
                return None;
            }
            if let Some(entries) = description.get("animation_controllers") {
                for entry in entries.as_array()? {
                    let entry = entry.as_object()?;
                    if entry.len() != 1 {
                        return None;
                    }
                    let (alias, target) = entry.iter().next()?;
                    if !target.as_str()?.starts_with("controller.animation.")
                        || !roots.controllers.insert(alias.clone())
                    {
                        return None;
                    }
                }
            }
        }
        "1.10.0" => {
            if description.get("animation_controllers").is_some() {
                return None;
            }
            if let Some(scripts) = description.get("scripts") {
                let scripts = scripts.as_object()?;
                if scripts.keys().any(|key| key != "animate") {
                    return None;
                }
                if let Some(entries) = scripts.get("animate") {
                    let mut seen = BTreeSet::new();
                    for entry in entries.as_array()? {
                        let alias = entry.as_str()?;
                        if !seen.insert(alias) {
                            return None;
                        }
                        let target = aliases?.get(alias)?.as_str()?;
                        if target.starts_with("controller.animation.") {
                            roots.controllers.insert(alias.to_owned());
                        } else if target.starts_with("animation.") {
                            roots.clips.insert(alias.to_owned());
                        } else {
                            return None;
                        }
                    }
                }
            }
        }
        _ => return None,
    }
    Some(roots)
}

/// One `scripts.animate` entry: an alias, optionally gated by a blend-weight expression.
pub(crate) struct ActivationRoot {
    pub alias: String,
    pub condition: Option<String>,
}

/// Returns the aliases a rig plays each tick, or `None` for an unrecognized schema.
pub(crate) fn activation_roots(value: &Value) -> Option<Vec<ActivationRoot>> {
    let description = value.get("minecraft:client_entity")?.get("description")?;
    let version = value.get("format_version")?.as_str()?;
    let mut parts = version.split('.').map(str::parse::<u32>);
    let (major, minor) = (parts.next()?.ok()?, parts.next()?.ok()?);
    let mut roots = Vec::<ActivationRoot>::new();
    let mut push = |alias: &str, condition: Option<&str>| {
        if !roots.iter().any(|root| root.alias == alias) {
            roots.push(ActivationRoot {
                alias: alias.to_owned(),
                condition: condition.map(str::to_owned),
            });
        }
    };
    if let Some(entries) = description.get("animation_controllers") {
        for entry in entries.as_array()? {
            for alias in entry.as_object()?.keys() {
                push(alias, None);
            }
        }
    }
    if (major, minor) >= (1, 10)
        && let Some(entries) = description.get("scripts").and_then(|v| v.get("animate"))
    {
        for entry in entries.as_array()? {
            match entry {
                Value::String(alias) => push(alias, None),
                Value::Object(weighted) if weighted.len() == 1 => {
                    let (alias, condition) = weighted.iter().next()?;
                    push(alias, Some(condition.as_str()?));
                }
                _ => return None,
            }
        }
    } else if (major, minor) < (1, 8) {
        return None;
    }
    Some(roots)
}

trait TransposeOption<T> {
    fn transpose_option(self) -> Option<Option<T>>;
}
impl<T> TransposeOption<T> for Option<Option<T>> {
    fn transpose_option(self) -> Option<Option<T>> {
        match self {
            None => Some(None),
            Some(v) => v.map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn aliases_do_not_activate_legacy_clips_and_explicit_modern_roots_are_preserved() {
        let legacy = json!({"format_version":"1.8.0","minecraft:client_entity":{"description":{"animations":{"swim":"animation.example"},"animation_controllers":[{"general":"controller.animation.example"}]}}});
        let roots = authored_roots(&legacy).unwrap();
        assert!(roots.clips.is_empty());
        assert!(roots.controllers.contains("general"));
        let modern = json!({"format_version":"1.10.0","minecraft:client_entity":{"description":{"animations":{"swim":"animation.example","general":"controller.animation.example","unused":"animation.unused"},"scripts":{"animate":["swim","general"]}}}});
        let roots = authored_roots(&modern).unwrap();
        assert_eq!(roots.len(), 2);
        assert!(roots.clips.contains("swim"));
        assert!(roots.controllers.contains("general"));
        assert!(!roots.clips.contains("unused"));
        let mut weighted = modern.clone();
        weighted["minecraft:client_entity"]["description"]["scripts"]["animate"] =
            json!([{"swim":"1"}]);
        assert!(authored_roots(&weighted).is_none());
    }
}
