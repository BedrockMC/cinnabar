//! Swing-weight sampling against frozen completed animation state.
use super::*;

/// Resamples weights on scratch controllers without committing transitions or advancing clip clocks.
pub(super) fn sample(
    evaluator: &evaluation::Evaluator<'_>,
    variables: &mut MolangVariables,
    state: &ActorRigState,
    previous: &[tick::WeightedClip],
    budget: &mut EvalBudget<'_>,
) -> Result<Vec<tick::WeightedClip>, EvalError> {
    let mut controllers = state.controllers.clone();
    let mut clips = tick::selection::select(
        evaluator,
        variables,
        &mut controllers,
        &state.clip_clocks,
        state.geometry_binding,
        super::super::skin_layers::blink_controller(evaluator.assets, state),
        budget,
    )?;
    for weighted in &mut clips {
        if let Some(old) = previous
            .iter()
            .find(|old| old.clip == weighted.clip && old.started_tick == weighted.started_tick)
        {
            weighted.time = old.time;
        } else if let Some(clock) = state
            .clip_clocks
            .get(&(weighted.clip, weighted.started_tick))
        {
            weighted.time = clock.time;
        } else {
            let clip = evaluator
                .assets
                .animation_clips()
                .get(weighted.clip)
                .ok_or(EvalError::Invalid)?;
            let raw_time = if clip.anim_time_update.is_some() {
                0.0
            } else {
                evaluator.anim_tick.saturating_sub(weighted.started_tick) as f32
                    * ANIMATION_TICK_SECONDS
            };
            let length = clip.length_seconds.get();
            weighted.time = match clip.loop_mode {
                assets::EntityAnimationLoop::Loop if length > 0.0 && raw_time > length => {
                    raw_time % length
                }
                assets::EntityAnimationLoop::HoldOnLastFrame => raw_time.min(length),
                _ => raw_time,
            };
        }
    }
    Ok(clips)
}
