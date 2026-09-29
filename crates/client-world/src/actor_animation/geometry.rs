use super::{evaluation::Evaluator, *};

/// Re-evaluates a multi-geometry rig's candidate conditions and, when another candidate now
/// holds (a baby grows up, a sheep is sheared), swaps the rig's bones and controllers to it.
pub(super) fn reselect_geometry(
    assets: &RuntimeEntityAssets,
    layout: &VariableLayout,
    state: &mut ActorRigState,
    actor: &ActorSnapshot,
    context: &ActorTickContext,
    budget: &mut EvalBudget<'_>,
) {
    let Some(rig) = assets.rig_bindings().get(state.rig_binding) else {
        return;
    };
    if rig.geometry_count < 2 {
        return;
    }
    let first = rig.first_geometry as usize;
    let Some(candidates) = assets
        .rig_geometries()
        .get(first..first + usize::from(rig.geometry_count))
    else {
        return;
    };
    let Some(input) = state.history.back().copied() else {
        return;
    };
    let evaluator = Evaluator {
        assets,
        layout,
        actor,
        input: &input,
        context,
        anim_tick: 0,
        life_tick: 0,
        finished: (false, false),
    };
    let mut variables = state.variables.clone();
    let mut selected = first;
    for (offset, candidate) in candidates.iter().enumerate().skip(1) {
        let Some(condition) = candidate.condition else {
            return;
        };
        match evaluator.run(condition as usize, &mut variables, 0.0, budget) {
            Ok(value) if value.truthy() => {
                selected = first + offset;
                break;
            }
            Ok(_) => {}
            Err(_) => return,
        }
    }
    if selected == state.geometry_binding {
        return;
    }
    let candidate = &assets.rig_geometries()[selected];
    if candidate.animation_count as usize + candidate.controller_count as usize
        > MAX_RUNTIME_BINDINGS_PER_RIG
    {
        return;
    }
    let Some((bones, bone_names)) = resolve_bones(assets, candidate.geometry as usize) else {
        return;
    };
    let Some(pose) = compose_pose(&bones, &[]) else {
        return;
    };
    let controller_first = candidate.first_controller as usize;
    let Some(bindings) = assets
        .rig_controllers()
        .get(controller_first..controller_first + usize::from(candidate.controller_count))
    else {
        return;
    };
    let mut controllers = Vec::new();
    for binding in bindings {
        if collect_controllers(assets, binding.controller as usize, 0, &mut controllers).is_none() {
            return;
        }
    }
    let rig_id = if state.pack {
        assets::PACK_RIG_ID_BASE.checked_add(selected as u32)
    } else {
        Some(selected as u32)
    };
    let Some(rig_id) = rig_id else {
        return;
    };
    state.rig = EntityRigId(rig_id);
    state.geometry_binding = selected;
    state.bones = bones;
    state.bone_names = bone_names;
    state.controllers = controllers;
    state.previous = pose.clone();
    state.rest = pose.clone();
    state.current = pose;
    state.reset_pending = true;
    state.rest_reset_pending = true;
}
