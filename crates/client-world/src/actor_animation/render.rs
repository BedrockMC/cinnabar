use super::{evaluation::Evaluator, *};

/// One texture layer a rig draws this tick, from its render controllers in controller order.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderTextureLayer {
    /// Entity-catalog source index of the raster.
    pub source: u32,
    /// Multiplies the texture; white when the controller sets no colour.
    pub color: [f32; 4],
    /// Blended over the texture; alpha 0 when unset.
    pub overlay: [f32; 4],
    /// Bones this layer does not draw.
    pub hidden_bones: Arc<[u32]>,
}

/// `pattern` is lowercase with an optional leading and/or trailing `*`; bone names match
/// ignoring ASCII case. Runs per rule, bone and actor every tick, so it never allocates.
fn pattern_matches(pattern: &str, name: &str) -> bool {
    let (leading, rest) = match pattern.strip_prefix('*') {
        Some(rest) => (true, rest),
        None => (false, pattern),
    };
    let (trailing, core) = match rest.strip_suffix('*') {
        Some(core) => (true, core),
        None => (false, rest),
    };
    let (name, core) = (name.as_bytes(), core.as_bytes());
    let at = |start: usize| {
        name.get(start..start + core.len())
            .is_some_and(|window| window.eq_ignore_ascii_case(core))
    };
    match (leading, trailing) {
        (true, true) => (0..=name.len().saturating_sub(core.len())).any(at),
        (true, false) => name.len() >= core.len() && at(name.len() - core.len()),
        (false, true) => at(0),
        (false, false) => name.eq_ignore_ascii_case(core),
    }
}

fn color(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    components: Option<[u32; 4]>,
    default: [f32; 4],
    budget: &mut EvalBudget<'_>,
) -> Result<[f32; 4], EvalError> {
    let Some(components) = components else {
        return Ok(default);
    };
    let mut value = [0.0; 4];
    for (slot, expression) in value.iter_mut().zip(components) {
        let number = evaluator.number(expression as usize, variables, 0.0, budget)?;
        *slot = if number.is_finite() { number } else { 0.0 };
    }
    Ok(value)
}

/// Evaluates the rig's render controllers against the actor's Molang state.
pub(super) fn evaluate_render(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    rig_binding: usize,
    bone_names: &[Box<str>],
    budget: &mut EvalBudget<'_>,
) -> Result<Vec<RenderTextureLayer>, EvalError> {
    let assets = evaluator.assets;
    let render = assets.render_data();
    let mut output = Vec::new();
    for layer in assets.render_layers(rig_binding) {
        budget.charge_work()?;
        if let Some(condition) = layer.condition
            && !evaluator
                .run(condition as usize, variables, 0.0, budget)?
                .truthy()
        {
            continue;
        }
        let rules = render
            .visibility
            .get(
                layer.first_visibility as usize
                    ..layer.first_visibility as usize + usize::from(layer.visibility_count),
            )
            .ok_or(EvalError::Invalid)?;
        let mut hidden = vec![false; bone_names.len()];
        for rule in rules {
            let visible = evaluator
                .run(rule.condition as usize, variables, 0.0, budget)?
                .truthy();
            for (index, name) in bone_names.iter().enumerate() {
                if pattern_matches(&rule.pattern, name) {
                    hidden[index] = !visible;
                }
            }
        }
        let hidden_bones: Arc<[u32]> = hidden
            .iter()
            .enumerate()
            .filter(|(_, hidden)| **hidden)
            .map(|(index, _)| index as u32)
            .collect();
        let tint = color(evaluator, variables, layer.color, [1.0; 4], budget)?;
        let overlay = color(evaluator, variables, layer.overlay_color, [0.0; 4], budget)?;
        let slots = render
            .slots
            .get(
                layer.first_slot as usize
                    ..layer.first_slot as usize + usize::from(layer.slot_count),
            )
            .ok_or(EvalError::Invalid)?;
        for slot in slots {
            let candidates = render
                .candidates
                .get(
                    slot.first_candidate as usize
                        ..slot.first_candidate as usize + usize::from(slot.candidate_count),
                )
                .ok_or(EvalError::Invalid)?;
            for candidate in candidates {
                budget.charge_work()?;
                let selected = match candidate.condition {
                    None => true,
                    Some(condition) => evaluator
                        .run(condition as usize, variables, 0.0, budget)?
                        .truthy(),
                };
                if selected {
                    output.push(RenderTextureLayer {
                        source: candidate.source,
                        color: tint,
                        overlay,
                        hidden_bones: Arc::clone(&hidden_bones),
                    });
                    break;
                }
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::pattern_matches;

    #[test]
    fn star_matches_everything_and_a_trailing_star_matches_a_prefix() {
        assert!(pattern_matches("*", "head"));
        assert!(pattern_matches("leg*", "legfront"));
        assert!(pattern_matches("head", "head"));
        assert!(!pattern_matches("head", "headwear"));
        assert!(!pattern_matches("leg*", "arm"));
        assert!(pattern_matches("*saddle*", "leftSaddleStrap"));
        assert!(pattern_matches("*ear", "MuleEar"));
        assert!(!pattern_matches("*saddle*", "body"));
        assert!(pattern_matches("leg*", "LegFront") && pattern_matches("head", "HEAD"));
        assert!(pattern_matches("*", "") && !pattern_matches("*ear", "ar"));
    }
}
