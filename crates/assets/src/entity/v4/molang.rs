use crate::AssetError;

use super::super::{CompiledEntityAssets, invalid, validate_geometry_scalar, validate_identifier};
use super::{
    MAX_MOLANG_COLLECTION_ITEMS, MAX_MOLANG_COLLECTION_ITEMS_TOTAL, MAX_MOLANG_COLLECTIONS,
    MAX_MOLANG_EXPRESSIONS, MAX_MOLANG_OPS, MAX_MOLANG_OPS_PER_EXPRESSION, MAX_MOLANG_STACK_DEPTH,
    MolangOp, MolangSymbol, MolangSymbolKind, range_in_bounds, validate_flattened_ranges,
};

/// Reviewed query namespace, sorted. A query without runtime actor state evaluates to its idle
/// value of zero, which is only listed where zero is the vanilla idle reading.
pub const MOLANG_QUERIES: &[&str] = &[
    "query.anim_time",
    "query.blocking",
    "query.body_y_rotation",
    "query.cape_flap_amount",
    "query.facing_target_to_range_attack",
    "query.ground_speed",
    "query.has_cape",
    "query.has_dash_cooldown",
    "query.has_head_gear",
    "query.has_rider",
    "query.has_target",
    "query.head_x_rotation",
    "query.head_y_rotation",
    "query.is_admiring",
    "query.is_alive",
    "query.is_angry",
    "query.is_baby",
    "query.is_casting",
    "query.is_celebrating",
    "query.is_charged",
    "query.is_charging",
    "query.is_crawling",
    "query.is_croaking",
    "query.is_dancing",
    "query.is_delayed_attacking",
    "query.is_digging",
    "query.is_eating",
    "query.is_eating_mob",
    "query.is_emoting",
    "query.is_gliding",
    "query.is_in_lava",
    "query.is_in_water",
    "query.is_interested",
    "query.is_jump_goal_jumping",
    "query.is_laying_egg",
    "query.is_leashed",
    "query.is_levitating",
    "query.is_moving",
    "query.is_on_ground",
    "query.is_playing_dead",
    "query.is_pregnant",
    "query.is_resting",
    "query.is_riding",
    "query.is_roaring",
    "query.is_scared",
    "query.is_searching",
    "query.is_shaking_wetness",
    "query.is_sitting",
    "query.is_sleeping",
    "query.is_sneaking",
    "query.is_sniffing",
    "query.is_sonic_boom",
    "query.is_spectator",
    "query.is_sprinting",
    "query.is_stalking",
    "query.is_stunned",
    "query.is_swimming",
    "query.is_tamed",
    "query.item_is_charged",
    "query.life_time",
    "query.main_hand_item_max_duration",
    "query.main_hand_item_use_duration",
    "query.modified_distance_moved",
    "query.modified_move_speed",
    "query.position_delta",
    "query.target_x_rotation",
    "query.target_y_rotation",
    "query.timer_flag_1",
    "query.timer_flag_2",
    "query.timer_flag_3",
    "query.vertical_speed",
];

/// Interpolates from `start` toward `end` degrees along the shorter arc.
#[must_use]
pub fn molang_lerp_rotate(start: f32, end: f32, amount: f32) -> f32 {
    let delta = (end - start + 180.0).rem_euclid(360.0) - 180.0;
    start + delta * amount
}

/// Returns the operands an op consumes and its net stack change.
#[must_use]
pub const fn molang_op_stack_effect(op: &MolangOp) -> (usize, isize) {
    match op {
        MolangOp::Push(_)
        | MolangOp::LoadQuery(_)
        | MolangOp::LoadVariable(_)
        | MolangOp::LoadThis => (0, 1),
        MolangOp::Negate
        | MolangOp::Not
        | MolangOp::Abs
        | MolangOp::Ceil
        | MolangOp::Floor
        | MolangOp::Round
        | MolangOp::Sqrt
        | MolangOp::Sin
        | MolangOp::Cos
        | MolangOp::SelectCollection(_)
        | MolangOp::StoreVariable(_)
        | MolangOp::Coalesce(_)
        | MolangOp::CallQuery(_) => (1, 0),
        MolangOp::Pop => (1, -1),
        MolangOp::Select | MolangOp::Clamp | MolangOp::Lerp | MolangOp::LerpRotate => (3, -2),
        MolangOp::Add
        | MolangOp::Subtract
        | MolangOp::Multiply
        | MolangOp::Divide
        | MolangOp::Modulo
        | MolangOp::And
        | MolangOp::Or
        | MolangOp::Equal
        | MolangOp::NotEqual
        | MolangOp::Less
        | MolangOp::LessEqual
        | MolangOp::Greater
        | MolangOp::GreaterEqual
        | MolangOp::Min
        | MolangOp::Max
        | MolangOp::Pow => (2, -1),
    }
}

pub(super) fn validate_molang_payload(compiled: &CompiledEntityAssets) -> Result<(), AssetError> {
    if compiled.molang_symbols.len() > MAX_MOLANG_EXPRESSIONS
        || compiled.molang_expressions.len() > MAX_MOLANG_EXPRESSIONS
        || compiled.molang_ops.len() > MAX_MOLANG_OPS
        || compiled.molang_collections.len() > MAX_MOLANG_COLLECTIONS
        || compiled.molang_collection_items.len() > MAX_MOLANG_COLLECTION_ITEMS_TOTAL
    {
        return Err(invalid("Molang payload count exceeds bound"));
    }
    let mut previous: Option<(MolangSymbolKind, &str)> = None;
    for symbol in &compiled.molang_symbols {
        validate_molang_symbol(symbol)?;
        let key = (symbol.kind, symbol.identifier.as_ref());
        if previous.is_some_and(|value| value >= key) {
            return Err(invalid("Molang symbols are not strictly ordered"));
        }
        previous = Some(key);
    }
    for expression in &compiled.molang_expressions {
        if expression.op_count as usize > MAX_MOLANG_OPS_PER_EXPRESSION
            || expression.max_stack > MAX_MOLANG_STACK_DEPTH
            || !range_in_bounds(
                expression.first_op,
                u32::from(expression.op_count),
                compiled.molang_ops.len(),
            )
        {
            return Err(invalid("invalid Molang expression range or stack bound"));
        }
        let start = expression.first_op as usize;
        let end = start + expression.op_count as usize;
        validate_molang_stack(&compiled.molang_ops[start..end], expression.max_stack)?;
    }
    validate_flattened_ranges(
        compiled
            .molang_expressions
            .iter()
            .map(|expression| (expression.first_op, u32::from(expression.op_count))),
        compiled.molang_ops.len(),
        "Molang operation",
    )?;
    for op in &compiled.molang_ops {
        let valid = match *op {
            MolangOp::Push(value) => {
                validate_geometry_scalar(value)?;
                true
            }
            MolangOp::LoadQuery(symbol) | MolangOp::CallQuery(symbol) => {
                molang_symbol_has_kind(compiled, symbol, &[MolangSymbolKind::Query])
            }
            MolangOp::LoadVariable(symbol)
            | MolangOp::StoreVariable(symbol)
            | MolangOp::Coalesce(symbol) => molang_symbol_has_kind(
                compiled,
                symbol,
                &[MolangSymbolKind::Variable, MolangSymbolKind::Temporary],
            ),
            MolangOp::SelectCollection(collection) => {
                (collection as usize) < compiled.molang_collections.len()
            }
            _ => true,
        };
        if !valid {
            return Err(invalid("Molang operand kind or index is invalid"));
        }
    }
    for collection in &compiled.molang_collections {
        if collection.item_count == 0
            || collection.item_count as usize > MAX_MOLANG_COLLECTION_ITEMS
            || !range_in_bounds(
                collection.first_item,
                u32::from(collection.item_count),
                compiled.molang_collection_items.len(),
            )
        {
            return Err(invalid("invalid Molang collection range"));
        }
    }
    validate_flattened_ranges(
        compiled
            .molang_collections
            .iter()
            .map(|collection| (collection.first_item, u32::from(collection.item_count))),
        compiled.molang_collection_items.len(),
        "Molang collection item",
    )?;
    for item in &compiled.molang_collection_items {
        validate_geometry_scalar(item.value)?;
    }
    Ok(())
}

fn validate_molang_symbol(symbol: &MolangSymbol) -> Result<(), AssetError> {
    validate_identifier(&symbol.identifier)?;
    let valid = match symbol.kind {
        MolangSymbolKind::Name => !["query.", "variable.", "temp."]
            .iter()
            .any(|prefix| symbol.identifier.starts_with(prefix)),
        MolangSymbolKind::Query => MOLANG_QUERIES
            .binary_search(&symbol.identifier.as_ref())
            .is_ok(),
        MolangSymbolKind::Variable => valid_molang_slot(&symbol.identifier, "variable."),
        MolangSymbolKind::Temporary => valid_molang_slot(&symbol.identifier, "temp."),
    };
    if !valid {
        return Err(invalid("Molang symbol is outside the reviewed namespace"));
    }
    Ok(())
}

fn valid_molang_slot(identifier: &str, prefix: &str) -> bool {
    identifier.strip_prefix(prefix).is_some_and(|slot| {
        !slot.is_empty()
            && slot
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    })
}

pub(super) fn molang_symbol_has_kind(
    compiled: &CompiledEntityAssets,
    index: u32,
    permitted: &[MolangSymbolKind],
) -> bool {
    compiled
        .molang_symbols
        .get(index as usize)
        .is_some_and(|symbol| permitted.contains(&symbol.kind))
}

fn validate_molang_stack(ops: &[MolangOp], declared_max: u8) -> Result<(), AssetError> {
    let mut depth = 0usize;
    let mut observed_max = 0usize;
    for op in ops {
        let (required, delta) = molang_op_stack_effect(op);
        if depth < required {
            return Err(invalid("Molang expression stack underflows"));
        }
        depth = depth.saturating_add_signed(delta);
        observed_max = observed_max.max(depth);
        if observed_max > declared_max as usize {
            return Err(invalid("Molang expression exceeds its declared stack"));
        }
    }
    if depth != 1 {
        return Err(invalid(
            "Molang expression must leave exactly one stack value",
        ));
    }
    if observed_max != declared_max as usize {
        return Err(invalid("Molang expression declared stack is not exact"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reviewed_query_namespace_is_sorted_and_unique() {
        assert!(MOLANG_QUERIES.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
