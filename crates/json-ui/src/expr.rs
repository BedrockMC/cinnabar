//! Length expressions for `size`/`offset`/`max_size`/`min_size`. A value is a sum
//! of unit terms (`100% - 4px`, `56.25%x - 65.25px + 118.5px`); the concrete
//! pixels of each unit come from an [`AxisContext`] the layout solver fills. `fill`
//! and `default` are standalone keywords the solver resolves against leftover space
//! and natural size. The boolean/string `view` grammar is evaluated by
//! `predicate.rs`, which T3 extends with binding lookup and string `+`.

use serde_json::Value;

/// A length unit. A bare or `px`-suffixed number is [`Unit::Px`]; the rest are the
/// percentage families JSON-UI recognises.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unit {
    Px,
    /// `%` — percent of the parent's length on this axis.
    Percent,
    /// `%c` — percent of the content extent of this control's children.
    PercentChildren,
    /// `%cm` — percent of the largest child on this axis.
    PercentChildrenMax,
    /// `%sm` — percent of the largest sibling on this axis.
    PercentSiblingMax,
    /// `%x` — percent of this control's own width.
    PercentX,
    /// `%y` — percent of this control's own height.
    PercentY,
}

/// One signed unit term of a length sum (`-65.25px`, `100%`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Term {
    /// Already-signed magnitude, e.g. `-65.25` for `- 65.25px`.
    pub coeff: f64,
    pub unit: Unit,
}

/// A parsed length: either a keyword or a sum of unit terms.
#[derive(Clone, Debug, PartialEq)]
pub enum Length {
    /// Absorbs the leftover main-axis space of a `stack_panel`; fills the parent
    /// axis elsewhere.
    Fill,
    /// The control's natural size (image `base_size`, label text extent), falling
    /// back to the parent axis when the control has no measurable content.
    Default,
    Terms(Vec<Term>),
}

/// The measured surroundings an axis expression evaluates against. Missing values
/// (a `%c` with unmeasured children) contribute zero.
#[derive(Clone, Copy, Debug, Default)]
pub struct AxisContext {
    pub parent: f64,
    pub own_width: Option<f64>,
    pub own_height: Option<f64>,
    pub children: Option<f64>,
    pub children_max: Option<f64>,
    pub sibling_max: Option<f64>,
    pub natural: Option<f64>,
}

/// The outcome of evaluating a length. `Fill` is deferred to the stack solver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Resolved {
    Pixels(f64),
    Fill,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExprError {
    #[error("empty length expression")]
    Empty,
    #[error("unexpected `{0}` in length expression")]
    Unexpected(String),
    #[error("missing number before unit in length expression")]
    MissingNumber,
    #[error("malformed number `{0}` in length expression")]
    BadNumber(String),
}

impl Length {
    /// `N%` of the parent axis.
    pub fn percent(value: f64) -> Self {
        Length::Terms(vec![Term {
            coeff: value,
            unit: Unit::Percent,
        }])
    }

    /// A fixed pixel length.
    pub fn pixels(value: f64) -> Self {
        Length::Terms(vec![Term {
            coeff: value,
            unit: Unit::Px,
        }])
    }

    /// Resolve to pixels; unit values absent from `ctx` count as zero. `fill`
    /// yields [`Resolved::Fill`]; `default` yields the natural size or, lacking
    /// one, the parent axis.
    pub fn eval(&self, ctx: &AxisContext) -> Resolved {
        match self {
            Length::Fill => Resolved::Fill,
            Length::Default => Resolved::Pixels(ctx.natural.unwrap_or(ctx.parent)),
            Length::Terms(terms) => {
                let pixels = terms
                    .iter()
                    .map(|term| term.coeff * factor(term.unit, ctx))
                    .sum();
                Resolved::Pixels(pixels)
            }
        }
    }

    /// Convenience: resolve, mapping `fill` to `parent` (its meaning outside a
    /// stack's main axis).
    pub fn eval_pixels(&self, ctx: &AxisContext) -> f64 {
        match self.eval(ctx) {
            Resolved::Pixels(value) => value,
            Resolved::Fill => ctx.parent,
        }
    }
}

fn factor(unit: Unit, ctx: &AxisContext) -> f64 {
    match unit {
        Unit::Px => 1.0,
        Unit::Percent => ctx.parent / 100.0,
        Unit::PercentChildren => ctx.children.unwrap_or(0.0) / 100.0,
        Unit::PercentChildrenMax => ctx.children_max.unwrap_or(0.0) / 100.0,
        Unit::PercentSiblingMax => ctx.sibling_max.unwrap_or(0.0) / 100.0,
        Unit::PercentX => ctx.own_width.unwrap_or(0.0) / 100.0,
        Unit::PercentY => ctx.own_height.unwrap_or(0.0) / 100.0,
    }
}

/// Parse a `size`/`offset` element: a number is pixels, a string is an expression.
/// A non-scalar element parses as `0px`.
pub fn length_from_value(value: &Value) -> Result<Length, ExprError> {
    match value {
        Value::Number(number) => Ok(Length::Terms(vec![Term {
            coeff: number.as_f64().unwrap_or(0.0),
            unit: Unit::Px,
        }])),
        Value::String(text) => parse_length(text),
        _ => Ok(Length::Terms(vec![Term {
            coeff: 0.0,
            unit: Unit::Px,
        }])),
    }
}

/// Parse a length string. `fill`/`default` are whole-string keywords; otherwise the
/// input is a `+`/`-` separated sum of `<number><unit>` terms.
pub fn parse_length(input: &str) -> Result<Length, ExprError> {
    let text = input.trim();
    match text {
        "" => return Err(ExprError::Empty),
        "fill" => return Ok(Length::Fill),
        "default" => return Ok(Length::Default),
        _ => {}
    }

    let bytes = text.as_bytes();
    let mut cursor = 0;
    let mut sign = read_leading_sign(bytes, &mut cursor);
    let mut terms = Vec::new();
    loop {
        skip_spaces(bytes, &mut cursor);
        let coeff = sign * read_number(text, bytes, &mut cursor)?;
        let unit = read_unit(bytes, &mut cursor);
        terms.push(Term { coeff, unit });
        skip_spaces(bytes, &mut cursor);
        match bytes.get(cursor) {
            None => break,
            Some(b'+') => sign = 1.0,
            Some(b'-') => sign = -1.0,
            Some(other) => return Err(ExprError::Unexpected((*other as char).to_string())),
        }
        cursor += 1;
    }
    Ok(Length::Terms(terms))
}

fn read_leading_sign(bytes: &[u8], cursor: &mut usize) -> f64 {
    skip_spaces(bytes, cursor);
    match bytes.get(*cursor) {
        Some(b'-') => {
            *cursor += 1;
            -1.0
        }
        Some(b'+') => {
            *cursor += 1;
            1.0
        }
        _ => 1.0,
    }
}

fn skip_spaces(bytes: &[u8], cursor: &mut usize) {
    while matches!(bytes.get(*cursor), Some(b' ' | b'\t')) {
        *cursor += 1;
    }
}

fn read_number(text: &str, bytes: &[u8], cursor: &mut usize) -> Result<f64, ExprError> {
    let start = *cursor;
    while matches!(bytes.get(*cursor), Some(byte) if byte.is_ascii_digit() || *byte == b'.') {
        *cursor += 1;
    }
    if *cursor == start {
        return Err(ExprError::MissingNumber);
    }
    let slice = &text[start..*cursor];
    slice
        .parse::<f64>()
        .map_err(|_| ExprError::BadNumber(slice.to_owned()))
}

/// Read the unit suffix after a number; longer keywords (`cm`, `sm`) win over the
/// single-letter forms.
fn read_unit(bytes: &[u8], cursor: &mut usize) -> Unit {
    if bytes.get(*cursor) == Some(&b'%') {
        *cursor += 1;
        return match (bytes.get(*cursor), bytes.get(*cursor + 1)) {
            (Some(b'c'), Some(b'm')) => {
                *cursor += 2;
                Unit::PercentChildrenMax
            }
            (Some(b's'), Some(b'm')) => {
                *cursor += 2;
                Unit::PercentSiblingMax
            }
            (Some(b'c'), _) => {
                *cursor += 1;
                Unit::PercentChildren
            }
            (Some(b'x'), _) => {
                *cursor += 1;
                Unit::PercentX
            }
            (Some(b'y'), _) => {
                *cursor += 1;
                Unit::PercentY
            }
            _ => Unit::Percent,
        };
    }
    if bytes.get(*cursor) == Some(&b'p') && bytes.get(*cursor + 1) == Some(&b'x') {
        *cursor += 2;
    }
    Unit::Px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terms(input: &str) -> Vec<Term> {
        match parse_length(input).unwrap() {
            Length::Terms(terms) => terms,
            other => panic!("expected terms, got {other:?}"),
        }
    }

    #[test]
    fn each_unit_parses_and_evaluates() {
        let ctx = AxisContext {
            parent: 200.0,
            own_width: Some(80.0),
            own_height: Some(45.0),
            children: Some(30.0),
            children_max: Some(12.0),
            sibling_max: Some(50.0),
            natural: Some(7.0),
        };
        assert_eq!(parse_length("4").unwrap().eval_pixels(&ctx), 4.0);
        assert_eq!(parse_length("4px").unwrap().eval_pixels(&ctx), 4.0);
        assert_eq!(parse_length("50%").unwrap().eval_pixels(&ctx), 100.0);
        assert_eq!(parse_length("100%c").unwrap().eval_pixels(&ctx), 30.0);
        assert_eq!(parse_length("50%cm").unwrap().eval_pixels(&ctx), 6.0);
        assert_eq!(parse_length("100%sm").unwrap().eval_pixels(&ctx), 50.0);
        assert_eq!(parse_length("50%x").unwrap().eval_pixels(&ctx), 40.0);
        assert_eq!(parse_length("100%y").unwrap().eval_pixels(&ctx), 45.0);
        assert_eq!(parse_length("default").unwrap().eval_pixels(&ctx), 7.0);
    }

    #[test]
    fn fill_is_distinct_from_pixels() {
        assert_eq!(
            parse_length("fill").unwrap().eval(&AxisContext::default()),
            Resolved::Fill
        );
    }

    #[test]
    fn default_falls_back_to_parent_without_natural() {
        let ctx = AxisContext {
            parent: 120.0,
            ..AxisContext::default()
        };
        assert_eq!(Length::Default.eval_pixels(&ctx), 120.0);
    }

    #[test]
    fn subtraction_and_addition_mix_units() {
        let ctx = AxisContext {
            parent: 100.0,
            children: Some(40.0),
            ..AxisContext::default()
        };
        assert_eq!(parse_length("100% - 4px").unwrap().eval_pixels(&ctx), 96.0);
        assert_eq!(parse_length("100%c + 6px").unwrap().eval_pixels(&ctx), 46.0);
    }

    #[test]
    fn aspect_ratio_expression_uses_own_width() {
        // 16:9 height from an already-known width, as vanilla thumbnails do.
        let ctx = AxisContext {
            own_width: Some(200.0),
            ..AxisContext::default()
        };
        let value = parse_length("56.25%x - 65.25px + 118.5px")
            .unwrap()
            .eval_pixels(&ctx);
        assert_eq!(value, 0.5625 * 200.0 - 65.25 + 118.5);
    }

    #[test]
    fn leading_negative_and_signed_terms() {
        assert_eq!(
            terms("-3px + 2px"),
            vec![
                Term {
                    coeff: -3.0,
                    unit: Unit::Px
                },
                Term {
                    coeff: 2.0,
                    unit: Unit::Px
                },
            ]
        );
    }

    #[test]
    fn cm_and_sm_win_over_c() {
        assert_eq!(terms("1%cm")[0].unit, Unit::PercentChildrenMax);
        assert_eq!(terms("1%sm")[0].unit, Unit::PercentSiblingMax);
        assert_eq!(terms("1%c")[0].unit, Unit::PercentChildren);
    }

    #[test]
    fn garbage_is_rejected() {
        assert_eq!(parse_length(""), Err(ExprError::Empty));
        assert!(matches!(
            parse_length("100% * 2"),
            Err(ExprError::Unexpected(_))
        ));
        assert_eq!(parse_length("%"), Err(ExprError::MissingNumber));
    }
}
