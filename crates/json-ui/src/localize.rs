//! Label localization as the client's `Localization::_get` applies it: text
//! without `%` is one whole key; otherwise each `%token` (ASCII letters,
//! digits, `-`, `.`, `_`) is replaced by its translation or, when missing, by
//! its own text, so an empty token drops its `%`. The character ending a token
//! is kept as is. Keys match exactly, then lowercased.

use std::{borrow::Cow, sync::Arc};

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
    let mut token: Option<usize> = None;
    for (at, ch) in text.char_indices() {
        match token {
            Some(_) if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_') => {}
            Some(start) => {
                substitute(&text[start..at], lookup, &mut out);
                out.push(ch);
                token = None;
            }
            None if ch == '%' => token = Some(at + 1),
            None => out.push(ch),
        }
    }
    if let Some(start) = token {
        substitute(&text[start..], lookup, &mut out);
    }
    Cow::Owned(out)
}

fn substitute(token: &str, lookup: &dyn Fn(&str) -> Option<Arc<str>>, out: &mut String) {
    match key(token, lookup) {
        Some(value) => out.push_str(&value),
        None => out.push_str(token),
    }
}

fn key(text: &str, lookup: &dyn Fn(&str) -> Option<Arc<str>>) -> Option<Arc<str>> {
    if text.is_empty() {
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
        assert_eq!(localize_text("100% sure", &table), "100 sure");
        assert_eq!(localize_text("%", &table), "");
        assert_eq!(
            localize_text("%menu.play%menu.play", &table),
            "Play%menu.play"
        );
        let long = format!("k{}", "a".repeat(256));
        let found = |key: &str| (key == long).then(|| Arc::from("long"));
        assert_eq!(localize_text(&long, &found), "long");
    }
}
