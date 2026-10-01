//! Binding-expression evaluator for `ignored`, `variables[]` `requires`, and the
//! data-binding `view` grammar, following the vanilla client's: `$var` and
//! `#binding` operands, quoted strings, bare int/float/bool literals, `not`,
//! `and`/`or` (one precedence level), `=`/`<`/`>`, `+`/`-`, and `*`/`/`, all
//! left-associative. On strings `+` concatenates, `-` removes every occurrence,
//! `/` counts occurrences, and a string result that reads as a number or bool
//! becomes one. An unbound `$var` reads as JSON null (false, `''`, 0), as
//! `UIEval::evalVariable` returns; an unbound `#binding` or anything outside the
//! grammar yields `None`, and the caller decides the lenient default.

use serde_json::Value;

use crate::env::Env;

/// Bounds on a server-supplied expression; beyond any of them it is undecidable.
/// Nesting (parentheses plus `not`) is what the recursive parser's stack depends on.
pub(crate) const MAX_BYTES: usize = 16 * 1024;
const MAX_TOKENS: usize = 4096;
pub(crate) const MAX_NESTING: usize = 64;

/// A resolved binding scalar: the value a `#name` lookup yields and the value a
/// `view` expression produces.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Bool(bool),
    Text(String),
    Num(f64),
}

impl Scalar {
    /// The boolean reading, honouring `"true"`/`"false"` text; `None` otherwise.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Scalar::Bool(value) => Some(*value),
            Scalar::Text(text) => match text.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            Scalar::Num(_) => None,
        }
    }
}

/// Resolves `#name` bindings for a `view` expression against a control's bound
/// values. The name is passed with its leading `#`.
pub trait Bindings {
    fn get(&self, name: &str) -> Option<Scalar>;
}

/// No binding scope: every `#name` is undecidable. Used by `ignored`/`requires`,
/// which never reference runtime bindings.
pub struct NoBindings;

impl Bindings for NoBindings {
    fn get(&self, _name: &str) -> Option<Scalar> {
        None
    }
}

/// Evaluate a boolean predicate with no binding scope, or `None` when undecidable.
pub fn eval(expression: &str, env: &Env) -> Option<bool> {
    eval_bool(expression, env, &NoBindings)
}

/// Evaluate a predicate to a boolean against `bindings`, or `None` when undecidable.
pub fn eval_bool(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<bool> {
    eval_operand(expression, env, bindings)?.as_bool()
}

/// Evaluate a `view` expression to its scalar result (a bool for `#visible`, text
/// for a concatenated `#texture`), or `None` when undecidable.
pub fn eval_scalar(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<Scalar> {
    eval_operand(expression, env, bindings).map(Operand::into_scalar)
}

fn eval_operand(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<Operand> {
    if expression.len() > MAX_BYTES {
        return None;
    }
    let tokens = tokenize(expression)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        depth: 0,
        env,
        bindings,
    };
    let value = parser.parse_or()?;
    (parser.pos == parser.tokens.len()).then_some(value)
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Not,
    And,
    Or,
    Eq,
    Less,
    Greater,
    LessEq,
    GreaterEq,
    Plus,
    Minus,
    Times,
    Divide,
    Str(String),
    Ident(String),
}

#[derive(Clone, Debug)]
enum Operand {
    Bool(bool),
    Str(String),
    Num(f64),
    /// An unbound `$var`: takes the other operand's empty value.
    Null,
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
            Operand::Num(number) => Some(*number != 0.0),
            Operand::Null => Some(false),
        }
    }

    /// Null as `like`'s empty value, so it compares and combines as jsoncpp's null.
    fn settle(self, like: &Operand) -> Operand {
        match (self, like) {
            (Operand::Null, Operand::Str(_)) => Operand::Str(String::new()),
            (Operand::Null, Operand::Num(_)) => Operand::Num(0.0),
            (Operand::Null, _) => Operand::Bool(false),
            (operand, _) => operand,
        }
    }

    fn as_str(&self) -> String {
        match self {
            Operand::Bool(value) => value.to_string(),
            Operand::Str(text) => text.clone(),
            Operand::Num(number) => format_number(*number),
            Operand::Null => String::new(),
        }
    }

    fn from_scalar(scalar: Scalar) -> Self {
        match scalar {
            Scalar::Bool(value) => Operand::Bool(value),
            Scalar::Text(text) => Operand::Str(text),
            Scalar::Num(number) => Operand::Num(number),
        }
    }

    fn into_scalar(self) -> Scalar {
        match self {
            Operand::Bool(value) => Scalar::Bool(value),
            Operand::Str(text) => Scalar::Text(text),
            Operand::Num(number) => Scalar::Num(number),
            Operand::Null => Scalar::Bool(false),
        }
    }
}

/// Render a number without a trailing `.0` so `#index` (a float) compares against a
/// `'2'` literal.
fn format_number(number: f64) -> String {
    if number.is_finite() && number.fract() == 0.0 {
        format!("{}", number as i64)
    } else {
        number.to_string()
    }
}

fn tokenize(expression: &str) -> Option<Vec<Token>> {
    let bytes = expression.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if tokens.len() >= MAX_TOKENS {
            return None;
        }
        let single = match bytes[i] {
            b'(' => Some(Token::Open),
            b')' => Some(Token::Close),
            b'=' => Some(Token::Eq),
            b'+' => Some(Token::Plus),
            b'*' => Some(Token::Times),
            b'/' => Some(Token::Divide),
            _ => None,
        };
        if let Some(token) = single {
            tokens.push(token);
            i += 1;
            continue;
        }
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            b'<' | b'>' => {
                let or_equal = bytes.get(i + 1) == Some(&b'=');
                tokens.push(match (bytes[i], or_equal) {
                    (b'<', false) => Token::Less,
                    (b'<', true) => Token::LessEq,
                    (_, false) => Token::Greater,
                    (_, true) => Token::GreaterEq,
                });
                i += 1 + usize::from(or_equal);
            }
            // A leading `-` on a number where an operand is expected is its sign.
            b'-' if !operand_ended(tokens.last())
                && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) =>
            {
                let start = i;
                i += 1;
                while i < bytes.len() && is_word_byte(bytes[i]) {
                    i += 1;
                }
                tokens.push(Token::Ident(expression[start..i].to_owned()));
            }
            b'-' => {
                tokens.push(Token::Minus);
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
            b'$' | b'#' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'.' | b'%' => {
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

fn operand_ended(last: Option<&Token>) -> bool {
    matches!(last, Some(Token::Close | Token::Str(_) | Token::Ident(_)))
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'$' | b'#' | b'_' | b'.' | b'|' | b'%')
}

/// Removes every non-overlapping occurrence of `needle` from `text`.
fn subtract_text(text: &str, needle: &str) -> String {
    if needle.is_empty() {
        text.to_owned()
    } else {
        text.replace(needle, "")
    }
}

/// A string result re-read as a literal: an int or float, a bool, else text.
fn reparse(text: String) -> Operand {
    if let Ok(number) = text.parse::<f64>()
        && !text.starts_with(['+', '-'])
    {
        return Operand::Num(number);
    }
    match text.as_str() {
        "true" => Operand::Bool(true),
        "false" => Operand::Bool(false),
        _ => Operand::Str(text),
    }
}

/// `"%.Ns"` picks the first N characters of the other operand.
fn truncation(format: &str) -> Option<usize> {
    format.strip_prefix("%.")?.strip_suffix('s')?.parse().ok()
}

/// `=`, `<`, `>` (and the `<=`/`>=` spellings) with the vanilla type rules.
fn compare(operator: &Token, left: &Operand, right: &Operand) -> Option<bool> {
    use Operand::{Bool, Null, Num, Str};
    Some(match operator {
        Token::Eq => match (left, right) {
            (Str(a), Str(b)) => a == b,
            (Num(a), Num(b)) => a == b,
            // A bool against text or a number compares truthiness, so an unset
            // (false) binding equals `''` as an empty controller string does.
            (Bool(_), _) | (_, Bool(_)) => {
                left.as_bool().unwrap_or_else(|| truthy(left))
                    == right.as_bool().unwrap_or_else(|| truthy(right))
            }
            // A numeric string compares as its number; other text against a
            // number is equal only when both read as false.
            (Str(text), Num(number)) | (Num(number), Str(text)) => match reparse(text.clone()) {
                Num(parsed) => parsed == *number,
                _ => !truthy(left) && !truthy(right),
            },
            // Unreachable after `settle`; null equals only an empty value.
            (Null, _) | (_, Null) => truthy(left) == truthy(right),
        },
        _ => {
            let ordering = match (left, right) {
                (Str(a), Str(b)) => a.cmp(b),
                (Num(a), Num(b)) => a.partial_cmp(b)?,
                _ => {
                    let (a, b) = (truthy(left), truthy(right));
                    return Some(match operator {
                        Token::Greater => a && !b,
                        Token::Less => !a && b,
                        Token::GreaterEq => a || !b,
                        _ => !a || b,
                    });
                }
            };
            match operator {
                Token::Less => ordering.is_lt(),
                Token::Greater => ordering.is_gt(),
                Token::LessEq => ordering.is_le(),
                _ => ordering.is_ge(),
            }
        }
    })
}

/// A binary operator's operands with any null settled against the other side.
fn settle(left: Operand, right: Operand) -> (Operand, Operand) {
    let left = left.settle(&right);
    let right = right.settle(&left);
    (left, right)
}

/// Truthiness for mixed comparisons: nonzero numbers and nonempty strings.
fn truthy(operand: &Operand) -> bool {
    match operand {
        Operand::Bool(value) => *value,
        Operand::Num(number) => *number != 0.0,
        Operand::Null => false,
        Operand::Str(text) => text.as_str() == "true" || (!text.is_empty() && text != "false"),
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    /// Open parentheses and `not`s enclosing the current position.
    depth: usize,
    env: &'a Env,
    bindings: &'a dyn Bindings,
}

/// `(...)` wrapping the whole text: an expression, not a string value.
fn is_expression(text: &str) -> bool {
    let text = text.trim();
    text.len() > 2 && text.starts_with('(') && text.ends_with(')')
}

impl Parser<'_> {
    /// Evaluate a variable's expression one level deeper, undecidable past the nesting bound.
    fn sub_expression(&self, text: &str) -> Option<Operand> {
        if self.depth >= MAX_NESTING || text.len() > MAX_BYTES {
            return None;
        }
        let tokens = tokenize(text)?;
        let mut parser = Parser {
            tokens: &tokens,
            pos: 0,
            depth: self.depth + 1,
            env: self.env,
            bindings: self.bindings,
        };
        let value = parser.parse_or()?;
        (parser.pos == parser.tokens.len()).then_some(value)
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    /// Runs one nested parse, undecidable once nesting reaches [`MAX_NESTING`].
    fn nested(&mut self, parse: fn(&mut Self) -> Option<Operand>) -> Option<Operand> {
        if self.depth >= MAX_NESTING {
            return None;
        }
        self.depth += 1;
        let value = parse(self);
        self.depth -= 1;
        value
    }

    fn parse_or(&mut self) -> Option<Operand> {
        let mut left = self.parse_not()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::And | Token::Or))
            .cloned()
        {
            self.pos += 1;
            let right = self.parse_not()?;
            let (a, b) = (left.as_bool()?, right.as_bool()?);
            left = Operand::Bool(if operator == Token::And {
                a && b
            } else {
                a || b
            });
        }
        Some(left)
    }

    fn parse_not(&mut self) -> Option<Operand> {
        if self.peek() == Some(&Token::Not) {
            self.pos += 1;
            let inner = self.nested(Self::parse_not)?;
            return Some(Operand::Bool(!inner.as_bool()?));
        }
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Option<Operand> {
        let mut left = self.parse_additive()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| {
                matches!(
                    token,
                    Token::Eq | Token::Less | Token::Greater | Token::LessEq | Token::GreaterEq
                )
            })
            .cloned()
        {
            self.pos += 1;
            let (settled, right) = settle(left, self.parse_additive()?);
            left = Operand::Bool(compare(&operator, &settled, &right)?);
        }
        Some(left)
    }

    fn parse_additive(&mut self) -> Option<Operand> {
        let mut left = self.parse_multiplicative()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Plus | Token::Minus))
            .cloned()
        {
            self.pos += 1;
            let right;
            (left, right) = settle(left, self.parse_multiplicative()?);
            left = match (operator, &left, &right) {
                (Token::Plus, Operand::Num(a), Operand::Num(b)) => Operand::Num(a + b),
                (Token::Plus, Operand::Str(_), _) | (Token::Plus, _, Operand::Str(_)) => {
                    reparse(format!("{}{}", left.as_str(), right.as_str()))
                }
                (Token::Plus, ..) => return None,
                (_, Operand::Num(a), Operand::Num(b)) => Operand::Num(a - b),
                (_, Operand::Str(a), Operand::Str(b)) => reparse(subtract_text(a, b)),
                // A string minus a number, or anything minus a string, keeps lhs.
                (_, Operand::Str(_), _) | (_, _, Operand::Str(_)) => left,
                _ => return None,
            };
        }
        Some(left)
    }

    fn parse_multiplicative(&mut self) -> Option<Operand> {
        let mut left = self.parse_atom()?;
        while let Some(operator) = self
            .peek()
            .filter(|token| matches!(token, Token::Times | Token::Divide))
            .cloned()
        {
            self.pos += 1;
            let right;
            (left, right) = settle(left, self.parse_atom()?);
            left = match (operator, &left, &right) {
                (Token::Times, Operand::Num(a), Operand::Num(b)) => Operand::Num(a * b),
                // A string times anything yields the right side as text (a
                // number reads as empty), cut to N characters by `'%.Ns'`.
                (Token::Times, Operand::Str(format), _) => {
                    let text = match &right {
                        Operand::Num(_) => String::new(),
                        other => other.as_str(),
                    };
                    reparse(match truncation(format) {
                        Some(count) => text.chars().take(count).collect(),
                        None => text,
                    })
                }
                (Token::Divide, Operand::Str(a), Operand::Str(b)) => {
                    Operand::Num(if b.is_empty() {
                        1.0
                    } else {
                        a.matches(b.as_str()).count() as f64
                    })
                }
                (Token::Divide, Operand::Num(a), Operand::Num(b)) if *b != 0.0 => {
                    Operand::Num(a / b)
                }
                (Token::Divide, Operand::Num(_), Operand::Num(_)) => left,
                _ => return None,
            };
        }
        Some(left)
    }

    fn parse_atom(&mut self) -> Option<Operand> {
        match self.tokens.get(self.pos)?.clone() {
            Token::Open => {
                self.pos += 1;
                let inner = self.nested(Self::parse_or)?;
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
            let name = name.split_once('|').map_or(name, |(name, _)| name);
            let Some(value) = self.env.get(name) else {
                return Some(Operand::Null);
            };
            return match value {
                Value::Null => Some(Operand::Null),
                Value::Bool(value) => Some(Operand::Bool(*value)),
                // A variable holding a parenthesised expression evaluates it, as
                // `$include_world_section: "($a and $b)"` does in vanilla.
                Value::String(text) if is_expression(text) => self.sub_expression(text),
                Value::String(text) => Some(Operand::Str(text.clone())),
                Value::Number(number) => number.as_f64().map(Operand::Num),
                _ => None,
            };
        }
        if word.starts_with('#') {
            // Runtime binding: resolved from the view's binding scope, or undecidable.
            return self.bindings.get(word).map(Operand::from_scalar);
        }
        if let Ok(number) = word.parse::<f64>() {
            return Some(Operand::Num(number));
        }
        Some(Operand::Str(word.to_owned()))
    }
}

#[cfg(test)]
mod expression_variable_tests {
    use super::eval;
    use crate::env::Env;
    use serde_json::json;

    // A variable holding an expression evaluates it; a self-reference stays undecidable.
    #[test]
    fn expression_variables_evaluate() {
        let mut env = Env::new();
        env.set("a", json!(false));
        env.set("b", json!(true));
        env.set("both", json!("($a and $b)"));
        env.set("loop", json!("(not $loop)"));
        assert_eq!(eval("(not $both)", &env), Some(true));
        assert_eq!(eval("$loop", &env), None);
        assert_eq!(eval("($a = '')", &env), Some(true));
    }
}

#[cfg(test)]
mod tests {
    use super::{Bindings, MAX_BYTES, MAX_NESTING, Scalar, eval, eval_bool, eval_scalar};
    use crate::env::Env;
    use serde_json::json;

    struct Map(std::collections::BTreeMap<String, Scalar>);

    impl Bindings for Map {
        fn get(&self, name: &str) -> Option<Scalar> {
            self.0.get(name).cloned()
        }
    }

    fn bindings(entries: &[(&str, Scalar)]) -> Map {
        Map(entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect())
    }

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
    // An unbound `$var` is null as in `UIEval::evalVariable`: the settings
    // screen's `((not $debug_settings) or $creator_build)` decides false.
    fn unbound_variable_is_null_and_binding_undecidable() {
        assert_eq!(eval("$never_set", &env()), Some(false));
        assert_eq!(eval("(not $never_set)", &env()), Some(true));
        assert_eq!(
            eval("($desktop_screen and $never_set)", &env()),
            Some(false)
        );
        assert_eq!(eval("($never_set = '')", &env()), Some(true));
        assert_eq!(eval("($never_set = 0)", &env()), Some(true));
        assert_eq!(eval("(not #visible)", &env()), None);
    }

    #[test]
    fn view_binding_drives_visibility() {
        // The vanilla dynamic_button image visibility rule.
        let present = bindings(&[("#texture", Scalar::Text("textures/x".into()))]);
        let empty = bindings(&[("#texture", Scalar::Text(String::new()))]);
        let loading = bindings(&[("#texture", Scalar::Text("loading".into()))]);
        let expr = "(not ((#texture = '') or (#texture = 'loading')))";
        assert_eq!(eval_bool(expr, &env(), &present), Some(true));
        assert_eq!(eval_bool(expr, &env(), &empty), Some(false));
        assert_eq!(eval_bool(expr, &env(), &loading), Some(false));
    }

    #[test]
    fn unbound_binding_stays_undecidable_under_a_scope() {
        let scope = bindings(&[("#other", Scalar::Bool(true))]);
        assert_eq!(eval_bool("(not (#texture = ''))", &env(), &scope), None);
    }

    #[test]
    fn concatenation_builds_a_texture_path() {
        let scope = bindings(&[("#name", Scalar::Text("apple".into()))]);
        let value = eval_scalar("'textures/items/' + #name", &env(), &scope);
        assert_eq!(value, Some(Scalar::Text("textures/items/apple".into())));
    }

    // Packs detect title markers by subtracting them and comparing.
    #[test]
    fn string_subtraction_detects_and_strips_markers() {
        let scope = bindings(&[("#t", Scalar::Text("@mineville/boxes:Spirit Bundle".into()))]);
        let strip = eval_scalar("(#t - '@mineville/boxes' - ':')", &env(), &scope);
        assert_eq!(strip, Some(Scalar::Text("Spirit Bundle".into())));
        let found = "(not ((#t - '@mineville/boxes') = #t))";
        assert_eq!(eval_bool(found, &env(), &scope), Some(true));
        let absent = "(not ((#t - '§j' - '§z') = #t))";
        assert_eq!(eval_bool(absent, &env(), &scope), Some(false));
    }

    #[test]
    fn arithmetic_and_ordering_follow_precedence() {
        let scope = bindings(&[("#n", Scalar::Num(7.0))]);
        assert_eq!(
            eval_scalar("(#n - 1) * 2 + 3", &env(), &scope),
            Some(Scalar::Num(15.0))
        );
        assert_eq!(
            eval_bool("(#n > 6) and (#n <= 7)", &env(), &scope),
            Some(true)
        );
        assert_eq!(
            eval_bool("true or false and false", &env(), &scope),
            Some(false)
        );
        assert_eq!(
            eval_scalar("-2 + #n", &env(), &scope),
            Some(Scalar::Num(5.0))
        );
        assert_eq!(
            eval_scalar("#n / 0", &env(), &scope),
            Some(Scalar::Num(7.0))
        );
    }

    #[test]
    fn string_operators_follow_the_vanilla_rules() {
        let none = bindings(&[]);
        let text = |value: &str| Some(Scalar::Text(value.into()));
        assert_eq!(
            eval_scalar("('a-b-a' / 'a')", &env(), &none),
            Some(Scalar::Num(2.0))
        );
        assert_eq!(
            eval_scalar("('%.3s' * 'abcdef')", &env(), &none),
            text("abc")
        );
        assert_eq!(
            eval_scalar("('x12' - 'x')", &env(), &none),
            Some(Scalar::Num(12.0))
        );
        assert_eq!(eval_scalar("('ab' - 3)", &env(), &none), text("ab"));
        assert_eq!(eval_scalar("100%", &env(), &none), text("100%"));
    }

    #[test]
    fn numeric_binding_compares_as_its_integer_text() {
        let scope = bindings(&[("#index", Scalar::Num(2.0))]);
        assert_eq!(eval_bool("(#index = '2')", &env(), &scope), Some(true));
        assert_eq!(eval_bool("(#index = '3')", &env(), &scope), Some(false));
    }

    // Server-supplied nesting must not overflow the stack.
    #[test]
    fn deep_parentheses_and_not_chains_are_undecidable() {
        let parens = format!("{}true{}", "(".repeat(20_000), ")".repeat(20_000));
        assert_eq!(eval(&parens, &env()), None);
        let nots = format!("{}true", "not ".repeat(20_000));
        assert_eq!(eval(&nots, &env()), None);
        let mixed = "(not ".repeat(20_000) + "true" + &")".repeat(20_000);
        assert_eq!(eval(&mixed, &env()), None);
    }

    #[test]
    fn nesting_and_length_within_the_bounds_still_evaluate() {
        let depth = MAX_NESTING - 1;
        let parens = format!("{}true{}", "(".repeat(depth), ")".repeat(depth));
        assert_eq!(eval(&parens, &env()), Some(true));
        let nots = format!("{}true", "not ".repeat(depth));
        assert_eq!(eval(&nots, &env()), Some(false));
        let long = format!("'{}' = ''", "x".repeat(MAX_BYTES + 1));
        assert_eq!(eval(&long, &env()), None);
    }
}
