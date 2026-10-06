//! Rawtext resolution against retained authoritative identity: score-owner
//! display names, the known player list, the pinned localization catalog,
//! and the local reader. Split from the runtime root to honor the
//! production line budget.

use std::{borrow::Cow, sync::Arc};

use super::UiRuntime;

impl UiRuntime {
    /// Refreshes the authoritative identities rawtext resolution reads: the
    /// display names of real score owners and the player-list usernames.
    /// Bounded by the retained score and player-list caps.
    pub fn refresh_raw_text_identities(
        &mut self,
        mut resolve_owner_name: impl FnMut(i64) -> Option<Arc<str>>,
        known_player_names: Vec<Arc<str>>,
    ) {
        self.score_owner_names = self
            .scoreboards
            .score_owner_ids()
            .into_iter()
            .filter_map(|unique_id| resolve_owner_name(unique_id).map(|name| (unique_id, name)))
            .collect();
        self.known_player_names = known_player_names;
    }

    /// The known player-list usernames.
    pub fn known_player_names(&self) -> &[Arc<str>] {
        &self.known_player_names
    }

    /// Momentary gameplay input; cleared along with the roster at session boundaries.
    pub fn set_player_list_held(&mut self, held: bool) {
        self.player_list_held = held;
    }

    pub fn player_list_held(&self) -> bool {
        self.player_list_held
    }

    /// Resolves one typed rawtext document against the retained scoreboard
    /// state, the pinned localization catalog, and the local reader
    /// identity. Score owners resolve through the authoritative
    /// id-to-display-name map (with the `*` reader sentinel handled by the
    /// resolver); selectors resolve only from retained authoritative state
    /// (`@s`, the player list for `@a`) and otherwise present as empty,
    /// counted. Nothing ever presents as JSON.
    pub(super) fn resolve_raw_text(
        &self,
        document: &protocol::RawTextDocument,
    ) -> protocol::ResolvedRawText {
        let translate = |key: &str| -> Option<Arc<str>> { self.translation(key) };
        let scoreboards = &self.scoreboards;
        let owner_names = &self.score_owner_names;
        let score = |owner: &str, objective: &str| {
            scoreboards.score_for_resolved_owner(objective, owner, |unique_id| {
                owner_names.get(&unique_id).cloned()
            })
        };
        let reader_name = &self.chat_source_name;
        let known_player_names = &self.known_player_names;
        let selector = |selector: &str| -> Option<Arc<str>> {
            match selector.trim() {
                "@s" => Some(Arc::clone(reader_name)).filter(|name| !name.is_empty()),
                "@a" if !known_player_names.is_empty() => {
                    let mut joined = String::new();
                    for (index, name) in known_player_names.iter().enumerate() {
                        if index > 0 {
                            joined.push_str(", ");
                        }
                        joined.push_str(name);
                    }
                    Some(Arc::from(joined))
                }
                // Position- or entity-dependent selectors need live queries
                // the retained state cannot answer authoritatively.
                _ => None,
            }
        };
        document.resolve_with_localized_arguments(
            &protocol::RawTextResolver {
                reader_name,
                translate: &translate,
                score: &score,
                selector: &selector,
            },
            &|text, max_bytes| localize_argument(text, &translate, max_bytes),
        )
    }
}

fn localize_argument(
    text: &str,
    translate: &dyn Fn(&str) -> Option<Arc<str>>,
    max_bytes: usize,
) -> Option<String> {
    match json_ui::localize_text_prefix(text, translate, max_bytes) {
        Cow::Borrowed(_) => None,
        Cow::Owned(localized) => Some(localized),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn rawtext_argument_expansion_bounds_catalog_work_and_retained_capacity() {
        let argument = "%x!".repeat(protocol::MAX_RAW_TEXT_OUTPUT_BYTES / "%x!".len());
        let document = protocol::parse_raw_text(&format!(
            r#"{{"rawtext":[{{"translate":"wrap","with":["{argument}"]}}]}}"#
        ))
        .unwrap();
        let expanded: Arc<str> = "x".repeat(1_024).into();
        let lookups = Cell::new(0);
        let translate = |key: &str| match key {
            "wrap" => Some(Arc::from("%s")),
            "x" => {
                lookups.set(lookups.get() + 1);
                Some(Arc::clone(&expanded))
            }
            _ => None,
        };
        let resolved = document.resolve_with_localized_arguments(
            &protocol::RawTextResolver {
                reader_name: "",
                translate: &translate,
                score: &|_, _| None,
                selector: &|_| None,
            },
            &|text, max_bytes| {
                let localized = localize_argument(text, &translate, max_bytes).unwrap();
                assert!(
                    localized.len() <= max_bytes,
                    "localized {} bytes for a {max_bytes}-byte prefix after {} catalog lookups",
                    localized.len(),
                    lookups.get()
                );
                assert!(localized.capacity() <= max_bytes);
                assert!(lookups.get() <= max_bytes / expanded.len() + 1);
                Some(localized)
            },
        );
        assert_eq!(resolved.text.len(), protocol::MAX_RAW_TEXT_OUTPUT_BYTES);
        assert!(resolved.truncated);
        assert!(resolved.text.bytes().enumerate().all(|(index, byte)| {
            byte == if index % (expanded.len() + 1) == expanded.len() {
                b'!'
            } else {
                b'x'
            }
        }));
    }

    #[test]
    fn rawtext_localized_arguments_preserve_omitted_scalar_truncation() {
        let document =
            protocol::parse_raw_text(r#"{"rawtext":[{"translate":"wrap","with":["%x"]}]}"#)
                .unwrap();
        for scalar in ["é", "世", "🌍"] {
            for remaining in 0..scalar.len() {
                let initial = "a".repeat(protocol::MAX_RAW_TEXT_OUTPUT_BYTES - remaining);
                let translated: Arc<str> = format!("{initial}{scalar}Z").into();
                let translate = |key: &str| match key {
                    "wrap" => Some(Arc::from("%s")),
                    "x" => Some(Arc::clone(&translated)),
                    _ => None,
                };
                let resolved = document.resolve_with_localized_arguments(
                    &protocol::RawTextResolver {
                        reader_name: "",
                        translate: &translate,
                        score: &|_, _| None,
                        selector: &|_| None,
                    },
                    &|text, max_bytes| localize_argument(text, &translate, max_bytes),
                );
                assert_eq!(resolved.text, initial);
                assert!(resolved.truncated);
            }
        }
    }
}
