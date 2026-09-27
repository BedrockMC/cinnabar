use assets::{AssetError, MOLANG_QUERIES, MolangSymbolKind};

use super::{Binary, Expr, Function, Unary, fold_binary, fold_function, fold_ternary, fold_unary};
use crate::entity::invalid;

const MAX_EXPRESSION_BYTES: usize = 16 * 1024;
const MAX_PARSE_DEPTH: usize = 32;
const MAX_STATEMENTS: usize = 64;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f32),
    Identifier(Box<str>),
    LeftParen,
    RightParen,
    Comma,
    Question,
    Colon,
    Semicolon,
    Assign,
    Operator(&'static str),
    End,
}

struct Lexer<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl Lexer<'_> {
    fn next(&mut self) -> Result<Token, AssetError> {
        while self
            .bytes
            .get(self.cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.cursor += 1;
        }
        let Some(&byte) = self.bytes.get(self.cursor) else {
            return Ok(Token::End);
        };
        let punctuation = match byte {
            b'(' => Some(Token::LeftParen),
            b')' => Some(Token::RightParen),
            b',' => Some(Token::Comma),
            b':' => Some(Token::Colon),
            b';' => Some(Token::Semicolon),
            _ => None,
        };
        if let Some(token) = punctuation {
            self.cursor += 1;
            return Ok(token);
        }
        for (text, operator) in [
            (b"??".as_slice(), "??"),
            (b"&&", "&&"),
            (b"||", "||"),
            (b"<=", "<="),
            (b">=", ">="),
            (b"==", "=="),
            (b"!=", "!="),
        ] {
            if self.bytes[self.cursor..].starts_with(text) {
                self.cursor += text.len();
                return Ok(Token::Operator(operator));
            }
        }
        if byte == b'?' {
            self.cursor += 1;
            return Ok(Token::Question);
        }
        if byte == b'=' {
            self.cursor += 1;
            return Ok(Token::Assign);
        }
        if let Some(operator) = match byte {
            b'+' => Some("+"),
            b'-' => Some("-"),
            b'*' => Some("*"),
            b'/' => Some("/"),
            b'%' => Some("%"),
            b'<' => Some("<"),
            b'>' => Some(">"),
            b'!' => Some("!"),
            _ => None,
        } {
            self.cursor += 1;
            return Ok(Token::Operator(operator));
        }
        if byte.is_ascii_digit() || byte == b'.' {
            return self.number();
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = self.cursor;
            self.cursor += 1;
            while self
                .bytes
                .get(self.cursor)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.'))
            {
                self.cursor += 1;
            }
            let identifier = std::str::from_utf8(&self.bytes[start..self.cursor])
                .map_err(|_| invalid("Molang identifier is not UTF-8"))?
                .to_ascii_lowercase();
            return Ok(Token::Identifier(expand_alias(identifier).into()));
        }
        Err(invalid("unsupported token in Molang expression"))
    }

    fn number(&mut self) -> Result<Token, AssetError> {
        let start = self.cursor;
        self.cursor += 1;
        while self.bytes.get(self.cursor).is_some_and(|byte| {
            byte.is_ascii_digit() || matches!(byte, b'.' | b'e' | b'E' | b'+' | b'-')
        }) {
            if matches!(self.bytes[self.cursor], b'+' | b'-')
                && !matches!(self.bytes[self.cursor - 1], b'e' | b'E')
            {
                break;
            }
            self.cursor += 1;
        }
        let text = std::str::from_utf8(&self.bytes[start..self.cursor])
            .map_err(|_| invalid("Molang number is not UTF-8"))?;
        // Authored float suffixes such as `1.0f` carry no Molang meaning.
        if self
            .bytes
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'f' | b'F'))
        {
            self.cursor += 1;
        }
        let value = text
            .parse::<f32>()
            .map_err(|_| invalid("invalid Molang numeric literal"))?;
        super::scalar(value)?;
        Ok(Token::Number(value))
    }
}

fn expand_alias(identifier: String) -> String {
    for (short, long) in [("q.", "query."), ("v.", "variable."), ("t.", "temp.")] {
        if let Some(rest) = identifier.strip_prefix(short) {
            return format!("{long}{rest}");
        }
    }
    identifier
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SymbolMode {
    /// Only the reviewed query namespace is accepted.
    Reviewed,
    /// Every query and variable reads as zero, for authoring-time default selection.
    Zero,
}

pub(super) struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    mode: SymbolMode,
}

impl<'a> Parser<'a> {
    pub(super) fn new(source: &'a str, mode: SymbolMode) -> Result<Self, AssetError> {
        if source.trim().is_empty() || source.len() > MAX_EXPRESSION_BYTES {
            return Err(invalid("Molang expression size exceeds bound"));
        }
        let mut lexer = Lexer {
            bytes: source.as_bytes(),
            cursor: 0,
        };
        let current = lexer.next()?;
        Ok(Self {
            lexer,
            current,
            mode,
        })
    }

    pub(super) fn parse(mut self) -> Result<Expr, AssetError> {
        let expression = self.parse_ternary(0)?;
        if self.current == Token::Semicolon {
            self.bump()?;
        }
        if self.current != Token::End {
            return Err(invalid("trailing or unsupported Molang syntax"));
        }
        Ok(expression)
    }

    /// Parses `statement (; statement)* ;?`, where a statement assigns or evaluates.
    pub(super) fn parse_statements(mut self) -> Result<Vec<Expr>, AssetError> {
        let mut statements = Vec::new();
        while self.current != Token::End {
            if statements.len() == MAX_STATEMENTS {
                return Err(invalid("Molang statement count exceeds bound"));
            }
            statements.push(self.parse_statement()?);
            match self.current {
                Token::Semicolon => {
                    self.bump()?;
                }
                Token::End => {}
                _ => return Err(invalid("Molang statements must be `;` separated")),
            }
        }
        if statements.is_empty() {
            return Err(invalid("empty Molang script"));
        }
        Ok(statements)
    }

    fn parse_statement(&mut self) -> Result<Expr, AssetError> {
        let expression = self.parse_ternary(0)?;
        if self.current != Token::Assign {
            return Ok(expression);
        }
        let Expr::Symbol(kind @ (MolangSymbolKind::Variable | MolangSymbolKind::Temporary), name) =
            expression
        else {
            return Err(invalid("Molang assignment target must be a variable"));
        };
        self.bump()?;
        let value = self.parse_ternary(0)?;
        Ok(Expr::Assign(kind, name, Box::new(value)))
    }

    fn bump(&mut self) -> Result<Token, AssetError> {
        let previous = std::mem::replace(&mut self.current, self.lexer.next()?);
        Ok(previous)
    }

    fn parse_ternary(&mut self, depth: usize) -> Result<Expr, AssetError> {
        check_depth(depth)?;
        let condition = self.parse_binary(0, depth + 1)?;
        if self.current != Token::Question {
            return Ok(condition);
        }
        self.bump()?;
        let yes = self.parse_ternary(depth + 1)?;
        if self.current != Token::Colon {
            return Err(invalid("Molang ternary is missing `:`"));
        }
        self.bump()?;
        let no = self.parse_ternary(depth + 1)?;
        fold_ternary(condition, yes, no)
    }

    fn parse_binary(&mut self, min_precedence: u8, depth: usize) -> Result<Expr, AssetError> {
        check_depth(depth)?;
        let mut left = self.parse_unary(depth + 1)?;
        loop {
            let Token::Operator(operator) = self.current else {
                break;
            };
            if operator == "??" {
                if min_precedence > 0 {
                    break;
                }
                self.bump()?;
                let fallback = self.parse_binary(1, depth + 1)?;
                left = match left {
                    Expr::Symbol(
                        kind @ (MolangSymbolKind::Variable | MolangSymbolKind::Temporary),
                        name,
                    ) => Expr::Coalesce(kind, name, Box::new(fallback)),
                    Expr::Constant(_) if self.mode == SymbolMode::Zero => left,
                    _ => return Err(invalid("Molang `??` requires a variable")),
                };
                continue;
            }
            let Some((precedence, binary)) = binary_operator(operator) else {
                break;
            };
            if precedence < min_precedence {
                break;
            }
            self.bump()?;
            let right = self.parse_binary(precedence + 1, depth + 1)?;
            left = fold_binary(binary, left, right)?;
        }
        Ok(left)
    }

    fn parse_unary(&mut self, depth: usize) -> Result<Expr, AssetError> {
        check_depth(depth)?;
        if let Token::Operator(operator @ ("-" | "!")) = self.current {
            self.bump()?;
            return fold_unary(
                if operator == "-" {
                    Unary::Negate
                } else {
                    Unary::Not
                },
                self.parse_unary(depth + 1)?,
            );
        }
        self.parse_primary(depth + 1)
    }

    fn parse_arguments(&mut self, depth: usize) -> Result<Vec<Expr>, AssetError> {
        self.bump()?;
        let mut arguments = Vec::new();
        if self.current != Token::RightParen {
            loop {
                arguments.push(self.parse_ternary(depth + 1)?);
                if self.current != Token::Comma {
                    break;
                }
                self.bump()?;
            }
        }
        if self.current != Token::RightParen {
            return Err(invalid("unclosed Molang call"));
        }
        self.bump()?;
        Ok(arguments)
    }

    fn parse_primary(&mut self, depth: usize) -> Result<Expr, AssetError> {
        check_depth(depth)?;
        match self.bump()? {
            Token::Number(value) => Ok(Expr::Constant(value)),
            Token::Identifier(identifier) if self.current == Token::LeftParen => {
                self.parse_call(&identifier, depth)
            }
            Token::Identifier(identifier) => self.parse_name(identifier),
            Token::LeftParen => {
                let expression = self.parse_ternary(depth + 1)?;
                if self.current != Token::RightParen {
                    return Err(invalid("unclosed Molang parenthesis"));
                }
                self.bump()?;
                Ok(expression)
            }
            _ => Err(invalid("expected Molang expression value")),
        }
    }

    fn parse_call(&mut self, identifier: &str, depth: usize) -> Result<Expr, AssetError> {
        if identifier.starts_with("query.") {
            let arguments = self.parse_arguments(depth)?;
            if self.mode == SymbolMode::Zero {
                return Ok(Expr::Constant(0.0));
            }
            if identifier != "query.position_delta" || arguments.len() != 1 {
                return Err(invalid("unsupported Molang query call"));
            }
            let argument = arguments.into_iter().next().expect("one checked argument");
            return Ok(Expr::CallQuery(identifier.into(), Box::new(argument)));
        }
        let function = parse_function(identifier)?;
        let arguments = self.parse_arguments(depth)?;
        if arguments.len() != function.arity() {
            return Err(invalid("Molang function has invalid arity"));
        }
        fold_function(function, arguments)
    }

    fn parse_name(&self, identifier: Box<str>) -> Result<Expr, AssetError> {
        match identifier.as_ref() {
            "true" => return Ok(Expr::Constant(1.0)),
            "false" => return Ok(Expr::Constant(0.0)),
            "this" => return Ok(Expr::This),
            "math.pi" => return Ok(Expr::Constant(std::f32::consts::PI)),
            _ => {}
        }
        let kind = if identifier.starts_with("query.") {
            if self.mode == SymbolMode::Reviewed
                && MOLANG_QUERIES.binary_search(&identifier.as_ref()).is_err()
            {
                return Err(invalid("unlisted Molang query"));
            }
            MolangSymbolKind::Query
        } else if identifier.starts_with("variable.") {
            validate_slot(&identifier, "variable.")?;
            MolangSymbolKind::Variable
        } else if identifier.starts_with("temp.") {
            validate_slot(&identifier, "temp.")?;
            MolangSymbolKind::Temporary
        } else {
            return Err(invalid("unlisted Molang identifier"));
        };
        if self.mode == SymbolMode::Zero {
            return Ok(Expr::Constant(0.0));
        }
        Ok(Expr::Symbol(kind, identifier))
    }
}

fn check_depth(depth: usize) -> Result<(), AssetError> {
    if depth > MAX_PARSE_DEPTH {
        Err(invalid("Molang parse depth exceeds bound"))
    } else {
        Ok(())
    }
}

fn validate_slot(identifier: &str, prefix: &str) -> Result<(), AssetError> {
    let valid = identifier.strip_prefix(prefix).is_some_and(|slot| {
        !slot.is_empty()
            && slot
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    });
    if valid {
        Ok(())
    } else {
        Err(invalid("invalid Molang variable or temporary slot"))
    }
}

fn parse_function(identifier: &str) -> Result<Function, AssetError> {
    match identifier {
        "math.abs" => Ok(Function::Abs),
        "math.ceil" => Ok(Function::Ceil),
        "math.floor" => Ok(Function::Floor),
        "math.round" => Ok(Function::Round),
        "math.sqrt" => Ok(Function::Sqrt),
        "math.sin" => Ok(Function::Sin),
        "math.cos" => Ok(Function::Cos),
        "math.min" => Ok(Function::Min),
        "math.max" => Ok(Function::Max),
        "math.clamp" => Ok(Function::Clamp),
        "math.lerp" => Ok(Function::Lerp),
        "math.pow" => Ok(Function::Pow),
        "math.mod" => Ok(Function::Mod),
        "math.lerprotate" => Ok(Function::LerpRotate),
        _ => Err(invalid("unsupported Molang function")),
    }
}

fn binary_operator(operator: &str) -> Option<(u8, Binary)> {
    Some(match operator {
        "||" => (1, Binary::Or),
        "&&" => (2, Binary::And),
        "==" => (3, Binary::Equal),
        "!=" => (3, Binary::NotEqual),
        "<" => (4, Binary::Less),
        "<=" => (4, Binary::LessEqual),
        ">" => (4, Binary::Greater),
        ">=" => (4, Binary::GreaterEqual),
        "+" => (5, Binary::Add),
        "-" => (5, Binary::Subtract),
        "*" => (6, Binary::Multiply),
        "/" => (6, Binary::Divide),
        "%" => (6, Binary::Modulo),
        _ => return None,
    })
}
