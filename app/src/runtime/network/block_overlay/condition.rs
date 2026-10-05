//! Per-state component resolution from permutation conditions.
//!
//! Only block-state/property equality conjunctions are evaluated, and
//! only when one state axis varies (sequential ids); hashed ids evaluate any
//! combination. Anything else is counted and left at base.

use protocol::{CustomBlock, CustomStateValue, CustomVisualComponents};

use super::OverlayGaps;

pub(super) fn state_components(
    block: &CustomBlock,
    state: u32,
    gaps: &mut OverlayGaps,
) -> CustomVisualComponents {
    let visuals = &block.visual;
    let mut resolved = visuals.base.clone();
    if visuals.permutations.is_empty() {
        return resolved;
    }
    let varying = visuals
        .state_axes
        .iter()
        .filter(|axis| axis.values.len() > 1)
        .collect::<Vec<_>>();
    let enumerable = varying.len() <= 1
        && varying.first().map_or(1, |axis| axis.values.len() as u64)
            == u64::from(block.state_count);
    if !enumerable {
        gaps.unevaluated_permutations += visuals.permutations.len() as u32;
        return resolved;
    }
    let state_value = |name: &str| {
        let axis = visuals
            .state_axes
            .iter()
            .find(|axis| axis.name.as_ref() == name)?;
        let index = if axis.values.len() > 1 {
            state as usize
        } else {
            0
        };
        axis.values.get(index)
    };
    apply_permutations(block, &state_value, &mut resolved, gaps);
    resolved
}

/// Components for one hashed-id state, whose `values` follow the block's
/// `state_axes`; every axis combination is evaluated exactly.
pub(super) fn assignment_components(
    block: &CustomBlock,
    values: &[CustomStateValue],
    gaps: &mut OverlayGaps,
) -> CustomVisualComponents {
    let visuals = &block.visual;
    let mut resolved = visuals.base.clone();
    let state_value = |name: &str| {
        let position = visuals
            .state_axes
            .iter()
            .position(|axis| axis.name.as_ref() == name)?;
        values.get(position)
    };
    apply_permutations(block, &state_value, &mut resolved, gaps);
    resolved
}

fn apply_permutations<'v>(
    block: &CustomBlock,
    state_value: &impl Fn(&str) -> Option<&'v CustomStateValue>,
    resolved: &mut CustomVisualComponents,
    gaps: &mut OverlayGaps,
) {
    for permutation in block.visual.permutations.iter() {
        match evaluate(&permutation.condition, state_value) {
            Some(true) => {
                let components = &permutation.components;
                if components.geometry.is_some() {
                    resolved.geometry.clone_from(&components.geometry);
                }
                if components.materials.is_some() {
                    resolved.materials.clone_from(&components.materials);
                }
                if components.transformation.is_some() {
                    resolved.transformation = components.transformation;
                }
                if components.light_dampening.is_some() {
                    resolved.light_dampening = components.light_dampening;
                }
                if components.light_emission.is_some() {
                    resolved.light_emission = components.light_emission;
                }
            }
            Some(false) => {}
            None => gaps.unevaluated_permutations += 1,
        }
    }
}

/// Evaluates `term && term ...`; `None` when any term is unsupported.
pub(super) fn evaluate<'a>(
    condition: &str,
    state_value: &impl Fn(&str) -> Option<&'a CustomStateValue>,
) -> Option<bool> {
    let mut result = true;
    for term in condition.split("&&") {
        result &= evaluate_term(strip_parens(term.trim()), state_value)?;
    }
    Some(result)
}

fn strip_parens(mut term: &str) -> &str {
    while let Some(inner) = term
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        term = inner.trim();
    }
    term
}

fn evaluate_term<'a>(
    term: &str,
    state_value: &impl Fn(&str) -> Option<&'a CustomStateValue>,
) -> Option<bool> {
    let (negate, left, right) = if let Some((left, right)) = term.split_once("==") {
        (false, left, right)
    } else {
        let (left, right) = term.split_once("!=")?;
        (true, left, right)
    };
    let left = left.trim();
    let name = [
        "q.block_state(",
        "query.block_state(",
        "q.block_property(",
        "query.block_property(",
    ]
    .into_iter()
    .find_map(|prefix| left.strip_prefix(prefix))?
    .strip_suffix(')')?;
    let actual = state_value(unquote(name.trim())?)?;
    let right = right.trim();
    let equal = match actual {
        CustomStateValue::String(value) => unquote(right)? == value.as_ref(),
        CustomStateValue::Int(value) => right.parse::<i64>().ok()? == *value,
        CustomStateValue::Bool(value) => match right {
            "true" | "1" => *value,
            "false" | "0" => !*value,
            _ => return None,
        },
    };
    Some(equal != negate)
}

fn unquote(text: &str) -> Option<&str> {
    text.strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .or_else(|| {
            text.strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
        })
}
