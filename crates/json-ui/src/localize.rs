//! Label localization as the vanilla client applies it (`Localization::_get`):
//! text without `%` is one whole key; otherwise each `%token` is replaced by its
//! translation, and a missing token draws without its `%`. Keys match exactly,
//! then lowercased. Unknown text draws verbatim.

use std::{borrow::Cow, sync::Arc};

/// Longest string worth a key lookup; lang keys are far shorter.
const MAX_KEY_BYTES: usize = 256;

/// `text` localized through `lookup` (the active language table).
pub fn localize_text<'a>(text: &'a str, lookup: &dyn Fn(&str) -> Option<Arc<str>>) -> Cow<'a, str> {
    if text.is_empty() {
        return Cow::Borrowed(text);
    }
    if !text.contains('%') {
        return match key(text, lookup) {
            Some(value) => Cow::Owned(value.to_string()),
            None => Cow::Borrowed(text),
        };
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let length = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_')))
            .unwrap_or(after.len());
        let token = &after[..length];
        if token.is_empty() {
            // A lone `%` is kept (vanilla's handling is unconfirmed).
            out.push('%');
        } else {
            match key(token, lookup) {
                Some(value) => out.push_str(&value),
                None => out.push_str(token),
            }
        }
        rest = &after[length..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn key(text: &str, lookup: &dyn Fn(&str) -> Option<Arc<str>>) -> Option<Arc<str>> {
    if text.len() > MAX_KEY_BYTES {
        return None;
    }
    lookup(text).or_else(|| {
        text.bytes()
            .any(|byte| byte.is_ascii_uppercase())
            .then(|| lookup(&text.to_ascii_lowercase()))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(key: &str) -> Option<Arc<str>> {
        match key {
            "menu.play" => Some("Play".into()),
            "trial.pausescreen.buygame" => Some("Unlock Full Game".into()),
            _ => None,
        }
    }

    #[test]
    fn whole_keys_tokens_and_unknown_text_follow_the_vanilla_rules() {
        assert_eq!(localize_text("menu.play", &table), "Play");
        assert_eq!(
            localize_text("trial.pauseScreen.buyGame", &table),
            "Unlock Full Game"
        );
        assert_eq!(localize_text("Hello there", &table), "Hello there");
        assert_eq!(localize_text("§l%menu.play!", &table), "§lPlay!");
        assert_eq!(localize_text("%missing.key x", &table), "missing.key x");
        assert_eq!(localize_text("100% sure", &table), "100% sure");
    }
}
