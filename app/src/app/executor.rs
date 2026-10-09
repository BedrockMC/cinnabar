//! Executor choice for the per-frame schedules of the main and render worlds.

use bevy::{
    app::{App, First, Last, PostUpdate, PreUpdate, Update},
    ecs::schedule::{ExecutorKind, ScheduleLabel},
    render::{Render, RenderApp},
};

/// Runs the main world's per-frame schedules and the render world's `Render` schedule on one
/// thread each. At several hundred frames per second the parallel executor's per-system task
/// wake-ups cost more than its parallelism returns, while meshing, lighting and skin preparation
/// already run on their own task pools.
pub(super) fn run_frame_schedules_on_one_thread(app: &mut App) {
    for label in frame_schedules() {
        app.edit_schedule(label, |schedule| {
            schedule.set_executor_kind(ExecutorKind::SingleThreaded);
        });
    }
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.edit_schedule(Render, |schedule| {
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
    use bevy::{app::SubApp, ecs::schedule::Schedule};

    use super::*;

    #[test]
    fn every_per_frame_schedule_runs_on_one_thread() {
        let mut app = App::new();
        let mut render_app = SubApp::new();
        render_app.add_schedule(Schedule::new(Render));
        app.insert_sub_app(RenderApp, render_app);
        run_frame_schedules_on_one_thread(&mut app);
        for label in frame_schedules() {
            let schedule = app.get_schedule(label).expect("main schedule exists");
            assert_eq!(schedule.get_executor_kind(), ExecutorKind::SingleThreaded);
        }
        let render = app
            .sub_app(RenderApp)
            .get_schedule(Render)
            .expect("render schedule exists");
        assert_eq!(render.get_executor_kind(), ExecutorKind::SingleThreaded);
    }
}
