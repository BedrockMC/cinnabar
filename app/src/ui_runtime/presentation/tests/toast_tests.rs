//! Toast rows remain independent from the gameplay HUD node count.

use std::sync::Arc;

use protocol::{HudEvent, PlayerGameMode, UiEvent};
use ui::{BoundedStat, DpiScale};

use crate::ui_runtime::{SequencedUiEvent, UiRuntime};

fn push_toast(runtime: &mut UiRuntime, fifo_sequence: u64) {
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence,
            local_millis: 0,
            server_tick: None,
            event: UiEvent::Hud(HudEvent::Toast {
                title: Arc::from("0"),
                message: Arc::from("2"),
            }),
        })
        .unwrap();
}

#[test]
fn populated_survival_hud_does_not_offset_or_reject_remote_toast_rows() {
    let Some(mut presentation) = super::engine_hud_tests::engine_presentation() else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    runtime.publish_player_game_mode(PlayerGameMode::Survival);
    runtime.hud.set_stats(
        BoundedStat::new(20, 20),
        BoundedStat::new(20, 20),
        BoundedStat::new(20, 20),
        None,
    );

    let baseline = presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    assert!(
        baseline.vertices.len() / 4 >= 60,
        "fixture must exercise the populated-HUD crash threshold"
    );

    push_toast(&mut runtime, 1);
    let with_toast = presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .expect("a routine remote toast must not make presentation fatal");
    // Border and fill quads plus a shadowed one-glyph title and message.
    assert_eq!(
        with_toast.vertices.len(),
        baseline.vertices.len() + 2 * 4 + 2 * 8
    );
}
