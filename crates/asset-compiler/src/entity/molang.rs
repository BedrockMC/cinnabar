use std::collections::{BTreeMap, BTreeSet, HashMap};

use assets::{
    AssetError, CompiledMolangExpression, EntityGeometryScalar, MAX_MOLANG_EXPRESSIONS,
    MAX_MOLANG_OPS, MAX_MOLANG_OPS_PER_EXPRESSION, MAX_MOLANG_STACK_DEPTH, MolangCollection,
    MolangCollectionItem, MolangOp, MolangSymbol, MolangSymbolKind, molang_op_stack_effect,
};

use super::invalid;

mod parser;

use parser::{Parser, SymbolMode};

#[derive(Clone, Default)]
pub(super) struct MolangCompiler {
    expressions: Vec<Expr>,
    interned: HashMap<Box<str>, u32>,
    names: BTreeSet<Box<str>>,
}

/// Compiler length before a speculative compile, restored when it fails.
#[derive(Clone, Copy)]
pub(super) struct MolangMark(usize);

pub(super) struct MolangPayload {
    pub symbols: Box<[MolangSymbol]>,
    pub expressions: Box<[CompiledMolangExpression]>,
    pub ops: Box<[MolangOp]>,
    pub collections: Box<[MolangCollection]>,
    pub collection_items: Box<[MolangCollectionItem]>,
}

impl MolangCompiler {
    pub fn compile(&mut self, source: &str) -> Result<u32, AssetError> {
        if let Some(&index) = self.interned.get(source) {
            return Ok(index);
        }
        let expression = Parser::new(source, SymbolMode::Reviewed)?.parse()?;
        let index = self.push(expression)?;
        self.interned.insert(source.into(), index);
        Ok(index)
    }

    /// Compiles one script from every supported statement; returns it with the dropped count.
    pub fn compile_script(&mut self, sources: &[&str]) -> Result<(Option<u32>, usize), AssetError> {
        let mut statements = Vec::new();
        let mut dropped = 0;
        for source in sources {
            match Parser::new(source, SymbolMode::Reviewed).and_then(Parser::parse_statements) {
                Ok(parsed) => statements.extend(parsed),
                Err(_) => dropped += 1,
            }
        }
        if statements.is_empty() {
            return Ok((None, dropped));
        }
        let script = if statements.len() == 1 {
            statements.pop().expect("one statement")
        } else {
            Expr::Sequence(statements)
        };
        match self.push(script) {
            Ok(index) => Ok((Some(index), dropped)),
            Err(_) => Ok((None, sources.len())),
        }
    }

    /// Evaluates a constant reading with every query and variable at zero.
    pub fn evaluate_default(source: &str) -> Option<f32> {
        match Parser::new(source, SymbolMode::Zero).ok()?.parse().ok()? {
            Expr::Constant(value) => Some(value),
            _ => None,
        }
    }

    pub fn add_name(&mut self, name: &str) -> Result<(), AssetError> {
        if name.is_empty() {
            return Err(invalid("empty Molang name"));
        }
        self.names.insert(name.into());
        Ok(())
    }

    pub fn mark(&self) -> MolangMark {
        MolangMark(self.expressions.len())
    }

    pub fn rollback(&mut self, mark: MolangMark) {
        self.expressions.truncate(mark.0);
        self.interned.retain(|_, index| (*index as usize) < mark.0);
    }

    fn push(&mut self, expression: Expr) -> Result<u32, AssetError> {
        if self.expressions.len() >= MAX_MOLANG_EXPRESSIONS {
            return Err(invalid("Molang expression count exceeds bound"));
        }
        let mut ops = 0;
        expression.count_ops(&mut ops);
        if ops > MAX_MOLANG_OPS_PER_EXPRESSION {
            return Err(invalid("Molang operation count exceeds bound"));
        }
        self.expressions.push(expression);
        Ok(self.expressions.len() as u32 - 1)
    }

    pub fn finish(self) -> Result<MolangPayload, AssetError> {
        let mut symbol_set = BTreeSet::new();
        for expression in &self.expressions {
            expression.collect_symbols(&mut symbol_set);
        }
        symbol_set.extend(
            self.names
                .into_iter()
                .map(|name| (MolangSymbolKind::Name, name)),
        );
        let symbols = symbol_set
            .into_iter()
            .map(|(kind, identifier)| MolangSymbol { kind, identifier })
            .collect::<Vec<_>>();
        let indices = symbols
            .iter()
            .enumerate()
            .map(|(index, symbol)| ((symbol.kind, symbol.identifier.clone()), index as u32))
            .collect::<BTreeMap<_, _>>();
        let mut ops = Vec::new();
        let mut expressions = Vec::with_capacity(self.expressions.len());
        for expression in self.expressions {
            let first_op = ops.len() as u32;
            expression.emit(&indices, &mut ops)?;
            let op_count = ops.len() - first_op as usize;
            if op_count == 0 || op_count > MAX_MOLANG_OPS_PER_EXPRESSION {
                return Err(invalid("Molang operation count exceeds bound"));
            }
            let max_stack = calculate_stack(&ops[first_op as usize..])?;
            expressions.push(CompiledMolangExpression {
                first_op,
                op_count: op_count as u16,
                max_stack,
            });
        }
        if ops.len() > MAX_MOLANG_OPS {
            return Err(invalid("total Molang operation count exceeds bound"));
        }

        Ok(MolangPayload {
            symbols: symbols.into_boxed_slice(),
            expressions: expressions.into_boxed_slice(),
            ops: ops.into_boxed_slice(),
            collections: Box::new([]),
            collection_items: Box::new([]),
        })
    }
}

#[derive(Clone, Copy)]
enum Unary {
    Negate,
    Not,
}

#[derive(Clone, Copy)]
enum Binary {
    Add,
    Subtract,
    Multiply,
    Divide,
    Modulo,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    And,
    Or,
}

#[derive(Clone, Copy)]
enum Function {
    Abs,
    Ceil,
    Floor,
    Round,
    Sqrt,
    Sin,
    Cos,
    Min,
    Max,
    Clamp,
    Lerp,
    Pow,
    Mod,
    LerpRotate,
}

#[derive(Clone)]
enum Expr {
    Constant(f32),
    Symbol(MolangSymbolKind, Box<str>),
    This,
    Unary(Unary, Box<Expr>),
    Binary(Binary, Box<Expr>, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Function(Function, Vec<Expr>),
    CallQuery(Box<str>, Box<Expr>),
    Coalesce(MolangSymbolKind, Box<str>, Box<Expr>),
    Assign(MolangSymbolKind, Box<str>, Box<Expr>),
    Sequence(Vec<Expr>),
}

impl Expr {
    fn children(&self) -> Vec<&Self> {
        match self {
            Self::Constant(_) | Self::Symbol(..) | Self::This => Vec::new(),
            Self::Unary(_, value)
            | Self::CallQuery(_, value)
            | Self::Coalesce(_, _, value)
            | Self::Assign(_, _, value) => vec![value],
            Self::Binary(_, left, right) => vec![left, right],
            Self::Ternary(condition, yes, no) => vec![condition, yes, no],
            Self::Function(_, values) | Self::Sequence(values) => values.iter().collect(),
        }
    }

    fn count_ops(&self, total: &mut usize) {
        *total += match self {
            Self::Sequence(values) => values.len().saturating_sub(1),
            _ => 1,
        };
        for child in self.children() {
            child.count_ops(total);
        }
    }

    fn collect_symbols(&self, symbols: &mut BTreeSet<(MolangSymbolKind, Box<str>)>) {
        match self {
            Self::Symbol(kind, identifier)
            | Self::Coalesce(kind, identifier, _)
            | Self::Assign(kind, identifier, _) => {
                symbols.insert((*kind, identifier.clone()));
            }
            Self::CallQuery(identifier, _) => {
                symbols.insert((MolangSymbolKind::Query, identifier.clone()));
            }
            _ => {}
        }
        for child in self.children() {
            child.collect_symbols(symbols);
        }
    }

    fn emit(
        &self,
        symbols: &BTreeMap<(MolangSymbolKind, Box<str>), u32>,
        output: &mut Vec<MolangOp>,
    ) -> Result<(), AssetError> {
        let index = |kind: MolangSymbolKind, identifier: &str| {
            symbols
                .get(&(kind, identifier.into()))
                .copied()
                .ok_or_else(|| invalid("Molang symbol was not interned"))
        };
        match self {
            Self::Constant(value) => output.push(MolangOp::Push(scalar(*value)?)),
            Self::This => output.push(MolangOp::LoadThis),
            Self::Symbol(kind, identifier) => {
                let symbol = index(*kind, identifier)?;
                output.push(match kind {
                    MolangSymbolKind::Query => MolangOp::LoadQuery(symbol),
                    MolangSymbolKind::Variable | MolangSymbolKind::Temporary => {
                        MolangOp::LoadVariable(symbol)
                    }
                    MolangSymbolKind::Name => return Err(invalid("name used as runtime value")),
                });
            }
            Self::Unary(operator, value) => {
                value.emit(symbols, output)?;
                output.push(match operator {
                    Unary::Negate => MolangOp::Negate,
                    Unary::Not => MolangOp::Not,
                });
            }
            Self::Binary(operator, left, right) => {
                left.emit(symbols, output)?;
                right.emit(symbols, output)?;
                output.push(operator.op());
            }
            Self::Ternary(condition, yes, no) => {
                condition.emit(symbols, output)?;
                yes.emit(symbols, output)?;
                no.emit(symbols, output)?;
                output.push(MolangOp::Select);
            }
            Self::Function(function, arguments) => {
                for argument in arguments {
                    argument.emit(symbols, output)?;
                }
                output.push(function.op());
            }
            Self::CallQuery(identifier, argument) => {
                argument.emit(symbols, output)?;
                output.push(MolangOp::CallQuery(index(
                    MolangSymbolKind::Query,
                    identifier,
                )?));
            }
            Self::Coalesce(kind, identifier, fallback) => {
                fallback.emit(symbols, output)?;
                output.push(MolangOp::Coalesce(index(*kind, identifier)?));
            }
            Self::Assign(kind, identifier, value) => {
                value.emit(symbols, output)?;
                output.push(MolangOp::StoreVariable(index(*kind, identifier)?));
            }
            Self::Sequence(statements) => {
                for (position, statement) in statements.iter().enumerate() {
                    if position > 0 {
                        output.push(MolangOp::Pop);
                    }
                    statement.emit(symbols, output)?;
                }
            }
        }
        if output.len() > MAX_MOLANG_OPS {
            return Err(invalid("total Molang operation count exceeds bound"));
        }
        Ok(())
    }
}

impl Binary {
    const fn op(self) -> MolangOp {
        match self {
            Self::Add => MolangOp::Add,
            Self::Subtract => MolangOp::Subtract,
            Self::Multiply => MolangOp::Multiply,
            Self::Divide => MolangOp::Divide,
            Self::Modulo => MolangOp::Modulo,
            Self::Less => MolangOp::Less,
            Self::LessEqual => MolangOp::LessEqual,
            Self::Greater => MolangOp::Greater,
            Self::GreaterEqual => MolangOp::GreaterEqual,
            Self::Equal => MolangOp::Equal,
            Self::NotEqual => MolangOp::NotEqual,
            Self::And => MolangOp::And,
            Self::Or => MolangOp::Or,
        }
    }
}

impl Function {
    const fn arity(self) -> usize {
        match self {
            Self::Abs
            | Self::Ceil
            | Self::Floor
            | Self::Round
            | Self::Sqrt
            | Self::Sin
            | Self::Cos => 1,
            Self::Min | Self::Max | Self::Pow | Self::Mod => 2,
            Self::Clamp | Self::Lerp | Self::LerpRotate => 3,
        }
    }

    const fn op(self) -> MolangOp {
        match self {
            Self::Abs => MolangOp::Abs,
            Self::Ceil => MolangOp::Ceil,
            Self::Floor => MolangOp::Floor,
            Self::Round => MolangOp::Round,
            Self::Sqrt => MolangOp::Sqrt,
            Self::Sin => MolangOp::Sin,
            Self::Cos => MolangOp::Cos,
            Self::Min => MolangOp::Min,
            Self::Max => MolangOp::Max,
            Self::Clamp => MolangOp::Clamp,
            Self::Lerp => MolangOp::Lerp,
            Self::Pow => MolangOp::Pow,
            Self::Mod => MolangOp::Modulo,
            Self::LerpRotate => MolangOp::LerpRotate,
        }
    }
}

fn fold_unary(operator: Unary, value: Expr) -> Result<Expr, AssetError> {
    if let Expr::Constant(value) = value {
        let result = match operator {
            Unary::Negate => -value,
            Unary::Not => bool_value(value == 0.0),
        };
        scalar(result)?;
        Ok(Expr::Constant(result))
    } else {
        Ok(Expr::Unary(operator, Box::new(value)))
    }
}

fn fold_binary(operator: Binary, left: Expr, right: Expr) -> Result<Expr, AssetError> {
    if let (Expr::Constant(left), Expr::Constant(right)) = (&left, &right) {
        let result = match operator {
            Binary::Add => left + right,
            Binary::Subtract => left - right,
            Binary::Multiply => left * right,
            Binary::Divide => {
                if *right == 0.0 {
                    0.0
                } else {
                    left / right
                }
            }
            Binary::Modulo => {
                if *right == 0.0 {
                    0.0
                } else {
                    left % right
                }
            }
            Binary::Less => bool_value(left < right),
            Binary::LessEqual => bool_value(left <= right),
            Binary::Greater => bool_value(left > right),
            Binary::GreaterEqual => bool_value(left >= right),
            Binary::Equal => bool_value(left == right),
            Binary::NotEqual => bool_value(left != right),
            Binary::And => bool_value(*left != 0.0 && *right != 0.0),
            Binary::Or => bool_value(*left != 0.0 || *right != 0.0),
        };
        scalar(result)?;
        Ok(Expr::Constant(result))
    } else {
        Ok(Expr::Binary(operator, Box::new(left), Box::new(right)))
    }
}

fn fold_ternary(condition: Expr, yes: Expr, no: Expr) -> Result<Expr, AssetError> {
    if let Expr::Constant(condition) = condition {
        Ok(if condition != 0.0 { yes } else { no })
    } else {
        Ok(Expr::Ternary(
            Box::new(condition),
            Box::new(yes),
            Box::new(no),
        ))
    }
}

fn fold_function(function: Function, arguments: Vec<Expr>) -> Result<Expr, AssetError> {
    let values = arguments
        .iter()
        .map(|argument| match argument {
            Expr::Constant(value) => Some(*value),
            _ => None,
        })
        .collect::<Option<Vec<_>>>();
    let Some(values) = values else {
        return Ok(Expr::Function(function, arguments));
    };
    let result = match function {
        Function::Abs => values[0].abs(),
        Function::Ceil => values[0].ceil(),
        Function::Floor => values[0].floor(),
        Function::Round => values[0].round(),
        Function::Sqrt => values[0].max(0.0).sqrt(),
        Function::Sin => values[0].to_radians().sin(),
        Function::Cos => values[0].to_radians().cos(),
        Function::Min => values[0].min(values[1]),
        Function::Max => values[0].max(values[1]),
        Function::Clamp => values[0].clamp(values[1].min(values[2]), values[1].max(values[2])),
        Function::Lerp => values[0] + (values[1] - values[0]) * values[2],
        Function::Pow => values[0].powf(values[1]),
        Function::Mod => {
            if values[1] == 0.0 {
                0.0
            } else {
                values[0] % values[1]
            }
        }
        Function::LerpRotate => assets::molang_lerp_rotate(values[0], values[1], values[2]),
    };
    if scalar(result).is_err() {
        return Ok(Expr::Function(
            function,
            values.into_iter().map(Expr::Constant).collect(),
        ));
    }
    Ok(Expr::Constant(result))
}

fn calculate_stack(ops: &[MolangOp]) -> Result<u8, AssetError> {
    let mut depth = 0usize;
    let mut maximum = 0usize;
    for op in ops {
        let (required, delta) = molang_op_stack_effect(op);
        if depth < required {
            return Err(invalid("compiled Molang stack underflows"));
        }
        depth = depth.saturating_add_signed(delta);
        maximum = maximum.max(depth);
    }
    if depth != 1 || maximum == 0 || maximum > MAX_MOLANG_STACK_DEPTH as usize {
        return Err(invalid("compiled Molang stack exceeds bound"));
    }
    Ok(maximum as u8)
}

fn scalar(value: f32) -> Result<EntityGeometryScalar, AssetError> {
    EntityGeometryScalar::new(value).ok_or_else(|| invalid("non-finite or excessive scalar"))
}

const fn bool_value(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}
