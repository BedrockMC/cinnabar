//! The vanilla UI expression evaluator (`UIEval::evalExpression` and
//! `UiExpression::evaluate`), shared by `ignored`, `requires`, folded property
//! expressions and binding `view` expressions. Operands are typed jsoncpp values:
//! bool, 32-bit int, 32-bit float, string, or null (an unset `$var`); `'` and `"`
//! quote strings. Unary `+`, `-` and `not` bind to the operand right after them;
//! binary operators reduce left to right by level: `*` `/`, then `+` `-`, then
//! `<` `>` `=`, then `and` `or`. An unbound `#binding` yields `None`, and the
//! caller decides the lenient default.

use serde_json::Value;

use crate::env::Env;

/// Longest server-supplied expression evaluated; evaluation itself is iterative.
pub(crate) const MAX_BYTES: usize = 1 << 20;

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

/// No binding scope: every `#name` is undecidable.
pub struct NoBindings;

impl Bindings for NoBindings {
    fn get(&self, _name: &str) -> Option<Scalar> {
        None
    }
}

/// Evaluate an expression with no binding scope to its jsoncpp truth value.
pub fn eval(expression: &str, env: &Env) -> Option<bool> {
    evaluate(expression, env, &NoBindings).map(|value| value.truth())
}

/// Evaluate a `view` expression to its scalar result, or `None` when undecidable.
pub fn eval_scalar(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<Scalar> {
    evaluate(expression, env, bindings).map(Operand::into_scalar)
}

/// Evaluate an expression with no `#binding` to its JSON value; `None` when it
/// needs a binding or is malformed, so the text stays for the binder.
pub fn eval_value(expression: &str, env: &Env) -> Option<Value> {
    evaluate(expression, env, &NoBindings).map(Operand::into_value)
}

/// A jsoncpp value as an expression token holds it.
#[derive(Clone, Debug, PartialEq)]
enum Operand {
    Null,
    Bool(bool),
    Int(i32),
    Float(f32),
    Str(String),
}

impl Operand {
    /// `Json::Value::asBool`: nonzero numbers and nonempty strings are true.
    fn truth(&self) -> bool {
        match self {
            Operand::Null => false,
            Operand::Bool(value) => *value,
            Operand::Int(value) => *value != 0,
            Operand::Float(value) => *value != 0.0,
            Operand::Str(text) => !text.is_empty(),
        }
    }

    fn as_int(&self) -> i32 {
        match self {
            Operand::Bool(value) => i32::from(*value),
            Operand::Int(value) => *value,
            Operand::Float(value) => *value as i32,
            Operand::Null | Operand::Str(_) => 0,
        }
    }

    /// `Json::Value::asFloat`: a string reads as zero.
    fn as_float(&self) -> f32 {
        match self {
            Operand::Bool(value) => f32::from(u8::from(*value)),
            Operand::Int(value) => *value as f32,
            Operand::Float(value) => *value,
            Operand::Null | Operand::Str(_) => 0.0,
        }
    }

    /// The text a string operator reads: strings and bools, anything else empty.
    fn text(&self) -> String {
        match self {
            Operand::Str(text) => text.clone(),
            Operand::Bool(value) => value.to_string(),
            _ => String::new(),
        }
    }

    fn is_str(&self) -> bool {
        matches!(self, Operand::Str(_))
    }

    fn from_json(value: &Value) -> Self {
        match value {
            Value::Null => Operand::Null,
            Value::Bool(value) => Operand::Bool(*value),
            Value::Number(number) => match number.as_i64().map(i32::try_from) {
                Some(Ok(value)) => Operand::Int(value),
                _ => Operand::Float(number.as_f64().unwrap_or(0.0) as f32),
            },
            Value::String(text) => Operand::Str(text.clone()),
            Value::Array(items) => Operand::Bool(!items.is_empty()),
            Value::Object(map) => Operand::Bool(!map.is_empty()),
        }
    }

    fn from_scalar(scalar: Scalar) -> Self {
        match scalar {
            Scalar::Bool(value) => Operand::Bool(value),
            Scalar::Text(text) => Operand::Str(text),
            Scalar::Num(number)
                if number.fract() == 0.0
                    && number >= f64::from(i32::MIN)
                    && number <= f64::from(i32::MAX) =>
            {
                Operand::Int(number as i32)
            }
            Scalar::Num(number) => Operand::Float(number as f32),
        }
    }

    fn into_scalar(self) -> Scalar {
        match self {
            Operand::Null => Scalar::Bool(false),
            Operand::Bool(value) => Scalar::Bool(value),
            Operand::Int(value) => Scalar::Num(f64::from(value)),
            Operand::Float(value) => Scalar::Num(f64::from(value)),
            Operand::Str(text) => Scalar::Text(text),
        }
    }

    fn into_value(self) -> Value {
        match self {
            Operand::Null => Value::Null,
            Operand::Bool(value) => Value::Bool(value),
            Operand::Int(value) => Value::from(value),
            Operand::Float(value) => {
                serde_json::Number::from_f64(f64::from(value)).map_or(Value::Null, Value::Number)
            }
            Operand::Str(text) => Value::String(text),
        }
    }
}

/// A string result re-read as a token literal: a quoted string, an int, a float,
/// a bool, else the text itself.
fn reparse(text: String) -> Operand {
    let bytes = text.as_bytes();
    if bytes.len() > 1 && matches!(bytes[0], b'\'' | b'"') && bytes[bytes.len() - 1] == bytes[0] {
        return Operand::Str(text[1..text.len() - 1].to_owned());
    }
    literal(text)
}

/// An unquoted word: an int, a float, a bool, else a string.
fn literal(word: String) -> Operand {
    if let Ok(value) = word.parse::<i32>() {
        return Operand::Int(value);
    }
    if word.bytes().any(|byte| byte.is_ascii_digit())
        && let Ok(value) = word.parse::<f32>()
    {
        return Operand::Float(value);
    }
    match word.as_str() {
        "true" => Operand::Bool(true),
        "false" => Operand::Bool(false),
        _ => Operand::Str(word),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    And,
    Or,
    Greater,
    Less,
    Eq,
    Plus,
    Minus,
    Times,
    Divide,
    Not,
}

impl Op {
    /// Binary level; a higher level reduces first.
    fn level(self) -> u8 {
        match self {
            Op::And | Op::Or => 1,
            Op::Greater | Op::Less | Op::Eq => 2,
            Op::Plus | Op::Minus => 3,
            Op::Times | Op::Divide => 4,
            Op::Not => 0,
        }
    }

    fn unary(self) -> bool {
        matches!(self, Op::Plus | Op::Minus | Op::Not)
    }
}

enum Item {
    Value(Operand),
    Op(Op),
}

fn is_delimiter(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t'
            | b'\n'
            | b'\r'
            | b'$'
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b'-'
            | b'/'
            | b'<'
            | b'='
            | b'>'
            | b'\''
            | b'"'
    )
}

/// Evaluate `expression`; `None` when malformed or a binding is unbound.
fn evaluate(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<Operand> {
    if expression.len() > MAX_BYTES {
        return None;
    }
    let bytes = expression.as_bytes();
    // One stack per open parenthesis.
    let mut frames: Vec<Vec<Item>> = vec![Vec::new()];
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let operand = match byte {
            b' ' | b'\t' | b'\n' | b'\r' => {
                i += 1;
                continue;
            }
            b'(' => {
                frames.push(Vec::new());
                i += 1;
                continue;
            }
            b')' => {
                let frame = frames.pop()?;
                if frames.is_empty() {
                    return None;
                }
                i += 1;
                finish(frame)?
            }
            b'\'' | b'"' => {
                let end = i + 1 + bytes[i + 1..].iter().position(|&next| next == byte)?;
                let text = expression[i + 1..end].to_owned();
                i = end + 1;
                Operand::Str(text)
            }
            b'*' | b'+' | b'-' | b'/' | b'<' | b'=' | b'>' => {
                let op = match byte {
                    b'*' => Op::Times,
                    b'+' => Op::Plus,
                    b'-' => Op::Minus,
                    b'/' => Op::Divide,
                    b'<' => Op::Less,
                    b'=' => Op::Eq,
                    _ => Op::Greater,
                };
                i += 1;
                push_op(frames.last_mut()?, op)?;
                continue;
            }
            _ => {
                let start = i;
                i += 1;
                while i < bytes.len() && !is_delimiter(bytes[i]) {
                    i += 1;
                }
                let word = &expression[start..i];
                match word {
                    "and" | "or" | "not" => {
                        let op = match word {
                            "and" => Op::And,
                            "or" => Op::Or,
                            _ => Op::Not,
                        };
                        push_op(frames.last_mut()?, op)?;
                        continue;
                    }
                    _ => word_operand(word, env, bindings)?,
                }
            }
        };
        push_value(frames.last_mut()?, operand)?;
    }
    if frames.len() != 1 {
        return None;
    }
    finish(frames.pop()?)
}

fn word_operand(word: &str, env: &Env, bindings: &dyn Bindings) -> Option<Operand> {
    if let Some(name) = word.strip_prefix('$') {
        return Some(match env.get(name) {
            // A variable naming a binding reads that binding.
            Some(Value::String(text)) if text.starts_with('#') => {
                Operand::from_scalar(bindings.get(text)?)
            }
            Some(value) => Operand::from_json(value),
            None => Operand::Null,
        });
    }
    if word.starts_with('#') {
        return bindings.get(word).map(Operand::from_scalar);
    }
    Some(literal(word.to_owned()))
}

fn push_op(frame: &mut Vec<Item>, op: Op) -> Option<()> {
    if matches!(frame.last(), Some(Item::Value(_))) {
        if op == Op::Not {
            return None;
        }
        reduce(frame, op.level())?;
    } else if !op.unary() {
        return None;
    }
    frame.push(Item::Op(op));
    Some(())
}

/// Push an operand, applying any unary operators written directly before it.
fn push_value(frame: &mut Vec<Item>, mut operand: Operand) -> Option<()> {
    if matches!(frame.last(), Some(Item::Value(_))) {
        return None;
    }
    while let Some(Item::Op(op)) = frame.last() {
        let op = *op;
        let at = frame.len() - 1;
        if !op.unary() || (at > 0 && !matches!(frame[at - 1], Item::Op(_))) {
            break;
        }
        frame.pop();
        operand = match op {
            Op::Not => Operand::Bool(!operand.truth()),
            Op::Minus => match operand {
                Operand::Float(value) => Operand::Float(-value),
                other => Operand::Int(other.as_int().wrapping_neg()),
            },
            _ => match operand {
                Operand::Float(value) => Operand::Float(value),
                other => Operand::Int(other.as_int()),
            },
        };
    }
    frame.push(Item::Value(operand));
    Some(())
}

/// Reduce `a op b` triples whose operator level is at least `level`.
fn reduce(frame: &mut Vec<Item>, level: u8) -> Option<()> {
    while frame.len() >= 3 {
        let at = frame.len();
        let Item::Op(op) = frame[at - 2] else {
            return None;
        };
        if op.level() < level || op == Op::Not {
            break;
        }
        let Some(Item::Value(right)) = frame.pop() else {
            return None;
        };
        frame.pop();
        let Some(Item::Value(left)) = frame.pop() else {
            return None;
        };
        frame.push(Item::Value(binary(op, left, right)));
    }
    Some(())
}

fn finish(mut frame: Vec<Item>) -> Option<Operand> {
    reduce(&mut frame, 1)?;
    match (frame.pop(), frame.is_empty()) {
        (Some(Item::Value(value)), true) => Some(value),
        _ => None,
    }
}

/// Removes every non-overlapping occurrence of `needle` from `text`.
fn subtract_text(text: &str, needle: &str) -> String {
    if needle.is_empty() {
        text.to_owned()
    } else {
        text.replace(needle, "")
    }
}

/// `"%.Ns"` picks the first N characters of the other operand.
fn truncation(format: &str) -> Option<usize> {
    format.strip_prefix("%.")?.strip_suffix('s')?.parse().ok()
}

fn number(
    left: &Operand,
    right: &Operand,
    int: fn(i32, i32) -> i32,
    float: fn(f32, f32) -> f32,
) -> Operand {
    if matches!(left, Operand::Float(_)) || matches!(right, Operand::Float(_)) {
        Operand::Float(float(left.as_float(), right.as_float()))
    } else {
        Operand::Int(int(left.as_int(), right.as_int()))
    }
}

/// One binary operator with the vanilla type rules.
fn binary(op: Op, left: Operand, right: Operand) -> Operand {
    use Operand::{Bool, Int, Str};
    match op {
        Op::And => Bool(left.truth() && right.truth()),
        Op::Or => Bool(left.truth() || right.truth()),
        Op::Greater | Op::Less => {
            let greater = op == Op::Greater;
            Bool(match (&left, &right) {
                (Str(a), Str(b)) => {
                    if greater {
                        a > b
                    } else {
                        a < b
                    }
                }
                _ if left.is_str() || right.is_str() => {
                    if greater {
                        left.truth() && !right.truth()
                    } else {
                        !left.truth() && right.truth()
                    }
                }
                _ if greater => left.as_float() > right.as_float(),
                _ => left.as_float() < right.as_float(),
            })
        }
        Op::Eq => Bool(match (&left, &right) {
            (Str(a), Str(b)) => a == b,
            (Str(_), _) => left.truth() == right.truth(),
            (Bool(_), _) | (_, Bool(_)) => left.truth() == right.truth(),
            _ => left.as_float() == right.as_float(),
        }),
        Op::Plus => match (&left, &right) {
            (Str(text), Int(value)) => reparse(format!("{text}{value}")),
            (Str(text), _) => reparse(format!("{text}{}", right.text())),
            (_, Str(text)) => reparse(format!("{}{text}", left.as_int())),
            _ => number(&left, &right, i32::wrapping_add, |a, b| a + b),
        },
        Op::Minus => match (&left, &right) {
            (Str(text), _) => reparse(subtract_text(text, &right.text())),
            (_, Str(_)) => left,
            _ => number(&left, &right, i32::wrapping_sub, |a, b| a - b),
        },
        Op::Times => match (&left, &right) {
            (Str(format), _) => {
                let text = right.text();
                reparse(match truncation(format) {
                    Some(count) => text.chars().take(count).collect(),
                    None => text,
                })
            }
            (_, Str(_)) => left,
            _ => number(&left, &right, i32::wrapping_mul, |a, b| a * b),
        },
        Op::Divide => match (&left, &right) {
            (Str(text), _) => {
                let needle = right.text();
                Int(if needle.is_empty() {
                    1
                } else {
                    text.matches(needle.as_str()).count() as i32
                })
            }
            (_, Str(_)) => left,
            // Dividing by zero keeps the left operand.
            _ if right.as_float() == 0.0 => left,
            _ => number(&left, &right, i32::wrapping_div, |a, b| a / b),
        },
        Op::Not => Operand::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::{Bindings, MAX_BYTES, Scalar, eval, eval_scalar, eval_value, evaluate};

    fn eval_bool(expression: &str, env: &Env, bindings: &dyn Bindings) -> Option<bool> {
        evaluate(expression, env, bindings).map(|value| value.truth())
    }
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

    fn value(expression: &str) -> Option<serde_json::Value> {
        eval_value(expression, &env())
    }

    #[test]
    fn variables_and_logic() {
        assert_eq!(eval("(not $use_custom_title_control)", &env()), Some(true));
        assert_eq!(eval("(not $show_close_button)", &env()), Some(false));
        assert_eq!(
            eval("$desktop_screen and (not $pocket_screen)", &env()),
            Some(true)
        );
        assert_eq!(
            eval("($pocket_screen or $desktop_screen)", &env()),
            Some(true)
        );
        assert_eq!(eval("($banner_text_binding_name = '')", &env()), Some(true));
        assert_eq!(eval("true or false and false", &env()), Some(false));
    }

    // An unbound `$var` is null; an unbound `#binding` is undecidable.
    #[test]
    fn unbound_variable_is_null_and_binding_undecidable() {
        assert_eq!(eval("$never_set", &env()), Some(false));
        assert_eq!(eval("(not $never_set)", &env()), Some(true));
        assert_eq!(eval("($never_set = '')", &env()), Some(true));
        assert_eq!(eval("($never_set = 0)", &env()), Some(true));
        assert_eq!(eval("(not #visible)", &env()), None);
        assert_eq!(value("(not #visible)"), None);
    }

    #[test]
    fn view_bindings_drive_visibility_and_text() {
        let present = bindings(&[("#texture", Scalar::Text("textures/x".into()))]);
        let loading = bindings(&[("#texture", Scalar::Text("loading".into()))]);
        let expr = "(not ((#texture = '') or (#texture = 'loading')))";
        assert_eq!(eval_bool(expr, &env(), &present), Some(true));
        assert_eq!(eval_bool(expr, &env(), &loading), Some(false));
        let scope = bindings(&[("#name", Scalar::Text("apple".into()))]);
        assert_eq!(
            eval_scalar("'textures/items/' + #name", &env(), &scope),
            Some(Scalar::Text("textures/items/apple".into()))
        );
    }

    #[test]
    fn string_operators_follow_the_vanilla_rules() {
        let scope = bindings(&[("#t", Scalar::Text("@mineville/boxes:Spirit Bundle".into()))]);
        let strip = eval_scalar("(#t - '@mineville/boxes' - ':')", &env(), &scope);
        assert_eq!(strip, Some(Scalar::Text("Spirit Bundle".into())));
        assert_eq!(value("('a-b-a' / 'a')"), Some(json!(2)));
        assert_eq!(value("('%.3s' * 'abcdef')"), Some(json!("abc")));
        assert_eq!(value("('x12' - 'x')"), Some(json!(12)));
        assert_eq!(value("100%"), Some(json!("100%")));
    }

    // Double-quoted operands and unary `+`/`-` parse.
    #[test]
    fn double_quotes_and_unary_arithmetic() {
        assert_eq!(value("(\"a\" = \"a\")"), Some(json!(true)));
        assert_eq!(value("(+ 2 = 2)"), Some(json!(true)));
        assert_eq!(value("(- (2) = -2)"), Some(json!(true)));
    }

    // Unary `not` binds to the operand after it, before any comparison.
    #[test]
    fn not_binds_tighter_than_comparison() {
        assert_eq!(value("(not 1 < 2)"), Some(json!(true)));
    }

    // Integers divide as integers; comparisons convert to 32-bit floats.
    #[test]
    fn numbers_keep_native_types() {
        assert_eq!(value("(3 / 2 = 1)"), Some(json!(true)));
        assert_eq!(value("(3 / 2)"), Some(json!(1)));
        assert_eq!(value("(3.0 / 2)"), Some(json!(1.5)));
        assert_eq!(value("(16777216 = 16777217)"), Some(json!(true)));
        assert_eq!(value("(7 / 0)"), Some(json!(7)));
    }

    // A nonempty string is true, whatever it spells.
    #[test]
    fn strings_use_json_truth() {
        assert_eq!(value("(not 'false')"), Some(json!(false)));
        assert_eq!(value("('abc' and true)"), Some(json!(true)));
    }

    // String results reparse as signed numbers; a number times a string keeps the number.
    #[test]
    fn string_results_reparse() {
        assert_eq!(value("(('x-2' - 'x') < 0)"), Some(json!(true)));
        assert_eq!(value("(2 * 'x' = 2)"), Some(json!(true)));
    }

    // Deep nesting evaluates iteratively; only absurd lengths are refused.
    #[test]
    fn deep_expressions_evaluate_without_recursion() {
        let parens = format!("{}true{}", "(".repeat(20_000), ")".repeat(20_000));
        assert_eq!(eval(&parens, &env()), Some(true));
        let nots = format!("{}true", "not ".repeat(20_001));
        assert_eq!(eval(&nots, &env()), Some(false));
        let long = format!("'{}' = ''", "x".repeat(MAX_BYTES + 1));
        assert_eq!(eval(&long, &env()), None);
        assert_eq!(eval("(true", &env()), None);
        assert_eq!(eval("true)", &env()), None);
    }
}
