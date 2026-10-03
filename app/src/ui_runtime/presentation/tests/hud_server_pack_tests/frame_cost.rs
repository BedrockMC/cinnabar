//! Offline timings through the installed font, server packs, HUD painter and scene publisher.

use super::*;
use std::time::{Duration, Instant};

#[test]
#[ignore = "release benchmark; needs local carriers and CINNABAR_FORM_PACK_DIR"]
fn offline_server_hud_publication_cost() {
    use super::super::super::forms::pack_harness;
    let Some(pack) = pack_harness::env_pack() else {
        eprintln!("HUD_PUBLICATION skipped: no offline pack directories");
        return;
    };
    let Some(mut presentation) = pack_harness::engine_presentation() else {
        eprintln!("HUD_PUBLICATION skipped: no installed carriers");
        return;
    };
    presentation.set_server_ui_pack(&pack);
    let mut player = crate::player_runtime::PlayerRuntime::new(1);
    let mut runtime = session(&mut player, "Zeqa lobby");
    runtime.set_session_glyphs(pack_harness::env_glyphs());
    let mut scene = render::UiRenderScene::default();
    let stats = render::UiRenderStats::default();
    for changing in [false, true] {
        let mut samples = Vec::with_capacity(500);
        let before = presentation.hud_passes();
        for index in 0_u64..=500 {
            let now = index * 8;
            if changing {
                runtime.hud.set_actionbar(
                    Arc::from(format!("Online: {} | Ping: {}ms", index % 7, index % 13)),
                    index + 100,
                    now,
                );
            }
            let started = Instant::now();
            let input = presentation
                .build(
                    &player,
                    &runtime,
                    now,
                    [2560, 1440],
                    DpiScale::new(2.0).unwrap(),
                )
                .unwrap();
            assert!(
                !input.vertices.is_empty(),
                "benchmark must publish visible HUD nodes"
            );
            scene.publish(input, &stats).unwrap();
            if index > 0 {
                samples.push(started.elapsed());
            }
        }
        report(changing, &mut samples, presentation.hud_passes() - before);
    }
}

/// Reports warm percentiles separately for idle paint and changing action-bar data.
fn report(changing: bool, samples: &mut [Duration], passes: usize) {
    samples.sort_unstable();
    eprintln!(
        "HUD_PUBLICATION changing={changing} n={} median_ms={:.3} p99_ms={:.3} bind_layout_passes={passes}",
        samples.len(),
        samples[249].as_secs_f64() * 1e3,
        samples[494].as_secs_f64() * 1e3
    );
}
