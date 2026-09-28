//! Boolean predicate evaluator for `ignored` and for `variables[]` `requires`.
//! It handles exactly what the vanilla pack uses: `$var` references, `not`/`and`/
//! `or`, parentheses, single-quoted string literals, and `=` string comparison.
//! Anything outside that grammar (an unbound variable, a `#binding`, an unknown
//! operator) yields `None`, and the caller decides the lenient default.

use serde_json::Value;

use crate::env::Env;

/// Evaluate a predicate to a boolean, or `None` when it cannot be decided.
pub fn eval(expression: &str, env: &Env) -> Option<bool> {
    let tokens = tokenize(expression)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        env,
    };
    let value = parser.parse_or()?;
    if parser.pos != parser.tokens.len() {
        return None;
    }
    value.as_bool()
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    Eq,
    Str(String),
    Ident(String),
}

#[derive(Clone, Debug)]
enum Operand {
    Bool(bool),
    Str(String),
}

impl Operand {
    fn as_bool(&self) -> Option<bool> {
        match self {
            Operand::Bool(value) => Some(*value),
            Operand::Str(text) => match text.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
        }
    }

    fn as_str(&self) -> String {
        match self {
            Operand::Bool(value) => value.to_string(),
            Operand::Str(text) => text.clone(),
        }
    }
}

fn tokenize(expression: &str) -> Option<Vec<Token>> {
    let bytes = expression.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            b')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            b'=' => {
                tokens.push(Token::Eq);
                i += 1;
            }
            b'\'' => {
                let start = i + 1;
                let mut end = start;
                while end < bytes.len() && bytes[end] != b'\'' {
                    end += 1;
                }
                if end >= bytes.len() {
                    return None;
                }
                tokens.push(Token::Str(expression[start..end].to_owned()));
                i = end + 1;
            }
            b'$' | b'#' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'%' | b'-' => {
                let start = i;
                while i < bytes.len() && is_word_byte(bytes[i]) {
                    i += 1;
                }
                let word = &expression[start..i];
                tokens.push(match word {
                    "not" => Token::Not,
                    "and" => Token::And,
                    "or" => Token::Or,
                    _ => Token::Ident(word.to_owned()),
                });
            }
            _ => return None,
        }
    }
    Some(tokens)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'$' | b'#' | b'_' | b'.' | b'%' | b'-')
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    env: &'a Env,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn parse_or(&mut self) -> Option<Operand> {
        let mut left = self.parse_and()?;
        while self.peek() == Some(&Token::Or) {
            self.pos += 1;
            let right = self.parse_and()?;
            left = Operand::Bool(left.as_bool()? || right.as_bool()?);
        }
        Some(left)
    }

    fn parse_and(&mut self) -> Option<Operand> {
        let mut left = self.parse_not()?;
        while self.peek() == Some(&Token::And) {
            self.pos += 1;
            let right = self.parse_not()?;
            left = Operand::Bool(left.as_bool()? && right.as_bool()?);
        }
        Some(left)
    }

    fn parse_not(&mut self) -> Option<Operand> {
        if self.peek() == Some(&Token::Not) {
            self.pos += 1;
            let inner = self.parse_not()?;
            return Some(Operand::Bool(!inner.as_bool()?));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Option<Operand> {
        let left = self.parse_atom()?;
        if self.peek() == Some(&Token::Eq) {
            self.pos += 1;
            let right = self.parse_atom()?;
            return Some(Operand::Bool(left.as_str() == right.as_str()));
        }
        Some(left)
    }

    fn parse_atom(&mut self) -> Option<Operand> {
        match self.tokens.get(self.pos)?.clone() {
            Token::Open => {
                self.pos += 1;
                let inner = self.parse_or()?;
                if self.peek() != Some(&Token::Close) {
                    return None;
                }
                self.pos += 1;
                Some(inner)
            }
            Token::Str(text) => {
                self.pos += 1;
                Some(Operand::Str(text))
            }
            Token::Ident(word) => {
                self.pos += 1;
                self.resolve_ident(&word)
            }
            _ => None,
        }
    }

    fn resolve_ident(&self, word: &str) -> Option<Operand> {
        match word {
            "true" => return Some(Operand::Bool(true)),
            "false" => return Some(Operand::Bool(false)),
            _ => {}
        }
        if let Some(name) = word.strip_prefix('$') {
            return match self.env.get(name)? {
                Value::Bool(value) => Some(Operand::Bool(*value)),
                Value::String(text) => Some(Operand::Str(text.clone())),
                Value::Number(number) => Some(Operand::Str(number.to_string())),
                _ => None,
            };
        }
        if word.starts_with('#') {
            // Runtime binding: undecidable at resolve time.
            return None;
        }
        Some(Operand::Str(word.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::eval;
    use crate::env::Env;
    use serde_json::json;

    fn env() -> Env {
        let mut env = Env::new();
        env.set("desktop_screen", json!(true));
        env.set("pocket_screen", json!(false));
        env.set("use_custom_title_control", json!(false));
        env.set("show_close_button", json!(true));
        env.set("banner_text_binding_name", json!(""));
        env
    }

    #[test]
    fn not_of_false_default_drops_the_control() {
        assert_eq!(eval("(not $use_custom_title_control)", &env()), Some(true));
    }

    #[test]
    fn plain_variable_and_negation() {
        assert_eq!(eval("$use_custom_title_control", &env()), Some(false));
        assert_eq!(eval("(not $show_close_button)", &env()), Some(false));
    }

    #[test]
    fn and_or_combinations() {
        assert_eq!(
            eval("$desktop_screen and (not $pocket_screen)", &env()),
            Some(true)
        );
        assert_eq!(
            eval("($pocket_screen or $desktop_screen)", &env()),
            Some(true)
        );
    }

    #[test]
    fn empty_string_comparison() {
        assert_eq!(eval("($banner_text_binding_name = '')", &env()), Some(true));
    }

    #[test]
    fn unknown_variable_and_binding_are_undecidable() {
        assert_eq!(eval("$never_set", &env()), None);
        assert_eq!(eval("(not #visible)", &env()), None);
    }
}
