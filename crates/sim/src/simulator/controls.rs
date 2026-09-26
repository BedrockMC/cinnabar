use super::{MovementInput, TickResult};

/// Fixed-tick primary controls, after item/pose slowdown but before the
/// simulation's movement impulse. Axes are left-positive strafe and forward.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProcessedControls {
    pub move_vector: [f64; 2],
}

/// A completed simulation and the exact controls used to produce it.
/// Keeping this separate preserves the historical trace/result schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlledTickResult {
    pub tick_result: TickResult,
    pub controls: ProcessedControls,
}

pub(super) fn process(input: MovementInput) -> ProcessedControls {
    let item = input
        .item_use_movement_modifier
        .unwrap_or(if input.using_consumable {
            super::CONSUMABLE_INPUT_MULTIPLIER
        } else {
            1.0
        });
    let factor = item
        * if input.sneaking {
            super::SNEAK_INPUT_MULTIPLIER
        } else {
            1.0
        };
    let process_axis = |axis: f64| {
        if input.move_vector_is_raw {
            axis.clamp(-1.0, 1.0) * factor
        } else {
            axis.clamp(-factor, factor)
        }
    };
    ProcessedControls {
        move_vector: [process_axis(input.strafe), process_axis(input.forward)],
    }
}
