use assets::{MolangSymbolKind, molang_lerp_rotate};

use super::*;

/// Dense variable and temporary slots of one carrier's Molang symbol table.
#[derive(Debug, Default)]
pub(super) struct VariableLayout {
    variable_base: usize,
    variable_count: usize,
    temp_base: usize,
    temp_count: usize,
    pub(super) engine: EngineSlots,
}

/// Slots of the variables the client, not the pack, assigns before `pre_animation`.
#[derive(Debug, Default)]
pub(super) struct EngineSlots {
    pub(super) attack_time: Option<usize>,
    pub(super) gliding_speed_value: Option<usize>,
    pub(super) is_first_person: Option<usize>,
    pub(super) player_x_rotation: Option<usize>,
    pub(super) is_holding_right: Option<usize>,
    pub(super) is_holding_left: Option<usize>,
}

impl VariableLayout {
    pub(super) fn new(assets: &RuntimeEntityAssets) -> Self {
        let symbols = assets.molang_symbols();
        let range = |kind: MolangSymbolKind| {
            let start = symbols.partition_point(|symbol| symbol.kind < kind);
            let end = symbols.partition_point(|symbol| symbol.kind <= kind);
            (start, end - start)
        };
        let (variable_base, variable_count) = range(MolangSymbolKind::Variable);
        let (temp_base, temp_count) = range(MolangSymbolKind::Temporary);
        let slot = |name: &str| {
            symbols[variable_base..variable_base + variable_count]
                .binary_search_by(|symbol| symbol.identifier.as_ref().cmp(name))
                .ok()
        };
        Self {
            variable_base,
            variable_count,
            temp_base,
            temp_count,
            engine: EngineSlots {
                attack_time: slot("variable.attack_time"),
                gliding_speed_value: slot("variable.gliding_speed_value"),
                is_first_person: slot("variable.is_first_person"),
                player_x_rotation: slot("variable.player_x_rotation"),
                is_holding_right: slot("variable.is_holding_right"),
                is_holding_left: slot("variable.is_holding_left"),
            },
        }
    }

    pub(super) fn fresh(&self) -> MolangVariables {
        MolangVariables {
            values: vec![0.0; self.variable_count],
            assigned: vec![false; self.variable_count],
            temps: vec![0.0; self.temp_count],
        }
    }
}

/// One actor's Molang variables; an unassigned variable reads as zero.
#[derive(Clone, Debug, Default)]
pub(super) struct MolangVariables {
    values: Vec<f32>,
    assigned: Vec<bool>,
    temps: Vec<f32>,
}

impl MolangVariables {
    pub(super) fn set(&mut self, slot: Option<usize>, value: f32) {
        if let Some(slot) = slot
            && slot < self.values.len()
        {
            self.values[slot] = value;
            self.assigned[slot] = true;
        }
    }

    pub(super) fn clear_temporaries(&mut self) {
        self.temps.fill(0.0);
    }

    fn slot(
        &mut self,
        layout: &VariableLayout,
        symbol: usize,
    ) -> Option<(&mut f32, Option<&mut bool>)> {
        if let Some(slot) = symbol.checked_sub(layout.variable_base)
            && slot < layout.variable_count
        {
            return Some((self.values.get_mut(slot)?, self.assigned.get_mut(slot)));
        }
        let slot = symbol.checked_sub(layout.temp_base)?;
        (slot < layout.temp_count).then_some(())?;
        Some((self.temps.get_mut(slot)?, None))
    }
}

/// Read-only inputs shared by every expression an actor evaluates in one tick.
pub(super) struct Evaluator<'a> {
    pub(super) assets: &'a RuntimeEntityAssets,
    pub(super) layout: &'a VariableLayout,
    pub(super) actor: &'a ActorSnapshot,
    pub(super) input: &'a ActorTickInput,
    pub(super) anim_tick: u64,
    pub(super) life_tick: u64,
}

impl Evaluator<'_> {
    /// Evaluates one compiled expression; `this` is the channel value earlier animations built.
    pub(super) fn run(
        &self,
        expression_index: usize,
        variables: &mut MolangVariables,
        this: f32,
        budget: &mut EvalBudget<'_>,
    ) -> Result<f32, EvalError> {
        let expression = self
            .assets
            .molang_expressions()
            .get(expression_index)
            .ok_or(EvalError::Invalid)?;
        let first = expression.first_op as usize;
        let end = first
            .checked_add(expression.op_count as usize)
            .ok_or(EvalError::Invalid)?;
        let ops = self
            .assets
            .molang_ops()
            .get(first..end)
            .ok_or(EvalError::Invalid)?;
        let mut stack = Vec::with_capacity(expression.max_stack as usize);
        for op in ops {
            budget.charge()?;
            self.step(*op, &mut stack, variables, this)?;
            if stack.last().is_some_and(|value| !value.is_finite()) {
                return Err(EvalError::Invalid);
            }
        }
        if stack.len() != 1 {
            return Err(EvalError::Invalid);
        }
        pop(&mut stack)
    }

    fn step(
        &self,
        op: MolangOp,
        stack: &mut Vec<f32>,
        variables: &mut MolangVariables,
        this: f32,
    ) -> Result<(), EvalError> {
        match op {
            MolangOp::Push(value) => stack.push(value.get()),
            MolangOp::LoadThis => stack.push(this),
            MolangOp::LoadQuery(symbol) => {
                let value = query::query(
                    self.actor,
                    self.input,
                    self.clock(),
                    self.symbol(symbol)?,
                    None,
                );
                stack.push(value);
            }
            MolangOp::CallQuery(symbol) => {
                let argument = pop(stack)?;
                let value = query::query(
                    self.actor,
                    self.input,
                    self.clock(),
                    self.symbol(symbol)?,
                    Some(argument),
                );
                stack.push(value);
            }
            MolangOp::LoadVariable(symbol) => {
                let value = variables
                    .slot(self.layout, symbol as usize)
                    .map_or(0.0, |(value, _)| *value);
                stack.push(value);
            }
            MolangOp::StoreVariable(symbol) => {
                let value = *stack.last().ok_or(EvalError::Invalid)?;
                let (slot, assigned) = variables
                    .slot(self.layout, symbol as usize)
                    .ok_or(EvalError::Invalid)?;
                *slot = value;
                if let Some(assigned) = assigned {
                    *assigned = true;
                }
            }
            MolangOp::Coalesce(symbol) => {
                let fallback = pop(stack)?;
                let value = match variables.slot(self.layout, symbol as usize) {
                    Some((value, Some(true))) => *value,
                    _ => fallback,
                };
                stack.push(value);
            }
            MolangOp::Pop => {
                pop(stack)?;
            }
            MolangOp::Add => binary(stack, |a, b| a + b)?,
            MolangOp::Subtract => binary(stack, |a, b| a - b)?,
            MolangOp::Multiply => binary(stack, |a, b| a * b)?,
            MolangOp::Divide => binary(stack, |a, b| if b == 0.0 { 0.0 } else { a / b })?,
            MolangOp::Modulo => binary(stack, |a, b| if b == 0.0 { 0.0 } else { a % b })?,
            MolangOp::Pow => binary(stack, f32::powf)?,
            MolangOp::Negate => unary(stack, |value| -value)?,
            MolangOp::Not => unary(stack, |value| bool_value(!truthy(value)))?,
            MolangOp::Abs => unary(stack, f32::abs)?,
            MolangOp::Ceil => unary(stack, f32::ceil)?,
            MolangOp::Floor => unary(stack, f32::floor)?,
            MolangOp::Round => unary(stack, f32::round)?,
            MolangOp::Sqrt => unary(stack, |value| value.max(0.0).sqrt())?,
            MolangOp::Sin => unary(stack, |value| value.to_radians().sin())?,
            MolangOp::Cos => unary(stack, |value| value.to_radians().cos())?,
            MolangOp::And => binary(stack, |a, b| bool_value(truthy(a) && truthy(b)))?,
            MolangOp::Or => binary(stack, |a, b| bool_value(truthy(a) || truthy(b)))?,
            MolangOp::Equal => binary(stack, |a, b| bool_value(a == b))?,
            MolangOp::NotEqual => binary(stack, |a, b| bool_value(a != b))?,
            MolangOp::Less => binary(stack, |a, b| bool_value(a < b))?,
            MolangOp::LessEqual => binary(stack, |a, b| bool_value(a <= b))?,
            MolangOp::Greater => binary(stack, |a, b| bool_value(a > b))?,
            MolangOp::GreaterEqual => binary(stack, |a, b| bool_value(a >= b))?,
            MolangOp::Min => binary(stack, f32::min)?,
            MolangOp::Max => binary(stack, f32::max)?,
            MolangOp::Select => ternary(
                stack,
                |condition, yes, no| {
                    if truthy(condition) { yes } else { no }
                },
            )?,
            MolangOp::Clamp => {
                let max = pop(stack)?;
                let min = pop(stack)?;
                let value = pop(stack)?;
                if min > max {
                    return Err(EvalError::Invalid);
                }
                stack.push(value.max(min).min(max));
            }
            MolangOp::Lerp => ternary(stack, |start, end, amount| start + (end - start) * amount)?,
            MolangOp::LerpRotate => ternary(stack, molang_lerp_rotate)?,
            MolangOp::SelectCollection(collection) => {
                let index = pop(stack)?;
                let collection = self
                    .assets
                    .molang_collections()
                    .get(collection as usize)
                    .ok_or(EvalError::Invalid)?;
                if collection.item_count == 0 {
                    return Err(EvalError::Invalid);
                }
                let clamped = index
                    .floor()
                    .clamp(0.0, f32::from(collection.item_count - 1))
                    as usize;
                let item = self
                    .assets
                    .molang_collection_items()
                    .get(collection.first_item as usize + clamped)
                    .ok_or(EvalError::Invalid)?;
                stack.push(item.value.get());
            }
        }
        Ok(())
    }

    const fn clock(&self) -> query::QueryClock {
        query::QueryClock {
            anim_tick: self.anim_tick,
            life_tick: self.life_tick,
        }
    }

    fn symbol(&self, index: u32) -> Result<&str, EvalError> {
        self.assets
            .molang_symbols()
            .get(index as usize)
            .map(|symbol| symbol.identifier.as_ref())
            .ok_or(EvalError::Invalid)
    }
}

pub(super) fn pop(stack: &mut Vec<f32>) -> Result<f32, EvalError> {
    stack.pop().ok_or(EvalError::Invalid)
}

fn unary(stack: &mut Vec<f32>, operation: impl FnOnce(f32) -> f32) -> Result<(), EvalError> {
    let value = pop(stack)?;
    stack.push(operation(value));
    Ok(())
}

fn binary(stack: &mut Vec<f32>, operation: impl FnOnce(f32, f32) -> f32) -> Result<(), EvalError> {
    let right = pop(stack)?;
    let left = pop(stack)?;
    stack.push(operation(left, right));
    Ok(())
}

fn ternary(
    stack: &mut Vec<f32>,
    operation: impl FnOnce(f32, f32, f32) -> f32,
) -> Result<(), EvalError> {
    let third = pop(stack)?;
    let second = pop(stack)?;
    let first = pop(stack)?;
    stack.push(operation(first, second, third));
    Ok(())
}

pub(super) fn truthy(value: f32) -> bool {
    value != 0.0
}

pub(super) fn bool_value(value: bool) -> f32 {
    u8::from(value).into()
}
