//! Executor choice for the main world's per-frame schedules.

use bevy::{
    app::{App, First, Last, PostUpdate, PreUpdate, Update},
    ecs::schedule::{ExecutorKind, ScheduleLabel},
};

/// Runs the main world's per-frame schedules on one thread. At several hundred frames per
/// second the parallel executor's per-system task wake-ups cost more than its parallelism
/// returns, while meshing, lighting and skin preparation already run on their own task pools.
pub(super) fn run_frame_schedules_on_one_thread(app: &mut App) {
    for label in frame_schedules() {
        app.edit_schedule(label, |schedule| {
            schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        });
    }
}

fn frame_schedules() -> [bevy::ecs::schedule::InternedScheduleLabel; 5] {
    [
        First.intern(),
        PreUpdate.intern(),
        Update.intern(),
        PostUpdate.intern(),
        Last.intern(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_per_frame_schedule_runs_on_one_thread() {
        let mut app = App::new();
        run_frame_schedules_on_one_thread(&mut app);
        for label in frame_schedules() {
            let schedule = app.get_schedule(label).expect("main schedule exists");
            assert_eq!(schedule.get_executor_kind(), ExecutorKind::SingleThreaded);
        }
    }
}
