//! Local-only: the engine HUD under real server resource packs, unpacked into
//! the `:`-separated directories `CINNABAR_HUD_PACK_DIRS` names (each is its
//! own session). Skips when unset or the UI carrier is absent; packs are never
//! committed.

use json_ui::{Draw, DrawNode};
use protocol::{PlayerGameMode, ScoreIdentity as ProtocolScoreIdentity};

use super::engine_hud_tests::engine_presentation;
use super::*;
use crate::ui_runtime::presentation::forms::pack_harness::dir_pack;

const PACK_ENV: &str = "CINNABAR_HUD_PACK_DIRS";

/// A populated session: stats, hotbar, sidebar, boss bar, title, and chat.
fn session(objective: &str) -> UiRuntime {
    let mut runtime = UiRuntime::new(1);
    runtime.publish_player_game_mode(PlayerGameMode::Survival);
    runtime.set_local_selected_slot(0);
    runtime.hud.set_stats(
        BoundedStat::new(20, 20),
        BoundedStat::new(20, 20),
        BoundedStat::new(20, 20),
        None,
    );
    runtime.hud.set_experience(12, 0.3);
    super::retained_hud_tests::install_mixed_scoreboard_slot(
        &mut runtime,
        "sidebar",
        &[
            (
                3,
                ProtocolScoreIdentity::FakePlayer(Arc::from("Kills: 4")),
                3,
            ),
            (
                4,
                ProtocolScoreIdentity::FakePlayer(Arc::from("zeqa.net")),
                2,
            ),
        ],
    );
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence: 10,
            local_millis: 0,
            server_tick: None,
            event: UiEvent::Objective(ObjectiveEvent::Display {
                display_slot: Arc::from("sidebar"),
                objective_name: Arc::from("objective"),
                display_name: Arc::from(objective),
                criteria_name: Arc::from("dummy"),
                sort_order: 1,
            }),
        })
        .unwrap();
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence: 11,
            local_millis: 0,
            server_tick: None,
            event: boss_event(
                ProtocolBossAction::Show,
                9,
                "Dragon",
                0.6,
                ProtocolBossColor::Pink,
                ProtocolBossOverlay::Progress,
            ),
        })
        .unwrap();
    for (sequence, line) in [(12, "hello"), (13, "toast.Welcome back")] {
        runtime
            .apply(SequencedUiEvent {
                session_id: 1,
                fifo_sequence: sequence,
                local_millis: 0,
                server_tick: None,
                event: chat_event(line),
            })
            .unwrap();
    }
    runtime.hud.set_title(Arc::from("Round 1"), 20, 0);
    runtime
}

fn textures(nodes: &[DrawNode]) -> std::collections::BTreeSet<&str> {
    nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Sprite { texture, .. } => Some(texture.as_str()),
            _ => None,
        })
        .collect()
}

fn texts(nodes: &[DrawNode]) -> Vec<&str> {
    nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Text { text, .. } if !text.is_empty() => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn server_packs_restyle_the_engine_hud() {
    let Ok(dirs) = std::env::var(PACK_ENV) else {
        return;
    };
    for dir in dirs.split(':').filter(|dir| !dir.is_empty()) {
        let Some(mut presentation) = engine_presentation() else {
            return;
        };
        presentation.set_server_ui_pack(&dir_pack(dir));
        let name = std::path::Path::new(dir)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let objective = if name.starts_with("eba25239") {
            "support.playhive.com/ui"
        } else {
            "Objective"
        };
        let runtime = session(objective);
        let started = std::time::Instant::now();
        presentation
            .build(&runtime, 500, [1920, 1080], DpiScale::new(1.0).unwrap())
            .unwrap();
        let first = started.elapsed();
        let nodes = presentation.hud_draw_nodes();
        eprintln!(
            "== {name}: {} nodes, first frame {first:?}\n   textures {:?}\n   texts {:?}",
            nodes.len(),
            textures(nodes),
            texts(nodes)
        );
        let drawn = textures(nodes);
        let written = texts(nodes);
        match name.get(..8).unwrap_or_default() {
            // Zeqa: its own sidebar art, no score column or title band, and
            // `toast.` chat lines drawn as its toasts instead of chat.
            "52e0000e" => {
                assert!(drawn.contains("textures/ui/zeqa/scoreboard/Black_sb"));
                assert!(!written.contains(&"3") && !written.contains(&"2"));
                assert!(written.contains(&"Kills: 4"));
                assert!(drawn.contains("textures/ui/zeqa/common/toastBorder"));
                assert!(drawn.contains("textures/ui/zeqa/common/scrollbar"));
            }
            // Hive: the flagged objective turns the sidebar into its entries.
            "eba25239" => {
                assert!(drawn.contains("textures/ui/hive/hive_scoreboard_entry"));
                assert!(written.contains(&"Kills: 4"));
            }
            _ => assert!(!nodes.is_empty()),
        }
    }
}
