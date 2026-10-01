//! Repeatable HUD bind/layout costs against the owner's local vanilla templates.

mod support;

use std::{hint::black_box, path::PathBuf, sync::Arc, time::Instant};

use json_ui::{
    BindState, BossBar, CachedLibrary, Catalog, CatalogLibrary, Context, HUD_SCREEN, HudModel,
    LayoutEnv, ResolveCache, Sidebar, TextMeasure, TextureMeta, TextureSource, Timed, ViewState,
    bind_shared, bind_stateful, hud_context, hud_data_source, render_bound, resolve,
};

/// Stable font metrics keep this benchmark independent of the rasterizer and GPU.
struct FixedText;

impl TextMeasure for FixedText {
    /// Measure the same nine-pixel lines on every run.
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 9.0]
    }
}

/// Fixed texture metadata avoids file reads inside the timed loop.
struct FixedTextures;

impl TextureSource for FixedTextures {
    /// Supply a stable size for every texture referenced by the templates.
    fn texture(&self, _path: &str) -> Option<TextureMeta> {
        Some(TextureMeta {
            base_size: [16.0; 2],
            nineslice: None,
        })
    }
}

#[test]
#[ignore = "benchmark; needs the local vanilla UI templates"]
fn frame_cost_bench_changing_hud_bind_layout() {
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let vanilla = support::vanilla_pack().join("ui");
    if !vanilla.is_dir() {
        eprintln!("FRAME_COST changing_hud: skipped, no local vanilla templates");
        return;
    }
    let mut catalog = Catalog::load_dir(&vanilla).unwrap();
    let java: Vec<_> = ["ui/hud_screen.json", "ui/scoreboards.json"]
        .into_iter()
        .map(|path| {
            (
                path,
                std::fs::read(base.join("assets/java-hud").join(path)).unwrap(),
            )
        })
        .collect();
    catalog.apply_pack(java.iter().map(|(path, bytes)| (*path, bytes.as_slice())));
    let context = hud_context(&Context::desktop());
    let started = Instant::now();
    let tree = Arc::new(resolve(&catalog, HUD_SCREEN, &context).control.unwrap());
    let cold_resolve = started.elapsed();
    let cache = ResolveCache::default();
    let library = CachedLibrary {
        library: CatalogLibrary {
            catalog: &catalog,
            context: &context,
        },
        cache: &cache,
    };
    let env = LayoutEnv {
        text: &FixedText,
        textures: &FixedTextures,
    };
    let mut model = HudModel {
        survival_ui: true,
        hotbar_visible: true,
        xp_bar: true,
        chat_visible: true,
        chat: (0..50)
            .map(|index| Timed {
                text: format!("player {index}: busy server chat"),
                born: index as f64,
            })
            .collect(),
        sidebar: Some(Sidebar {
            title: "Busy server".into(),
            rows: (0..15)
                .map(|index| (format!("Player {index}"), index.to_string()))
                .collect(),
            background_opacity: 0.3,
            title_background_opacity: 0.4,
        }),
        boss_bars: (0..8)
            .map(|index| BossBar {
                name: format!("Boss {index}"),
                progress: 0.5,
                color: "#aa00aa".into(),
                notches: 0,
            })
            .collect(),
        ..HudModel::default()
    };
    let mut bind_time = std::time::Duration::ZERO;
    // The host keeps live binding state across refreshes.
    let mut state = BindState::new();
    let mut stateful_time = std::time::Duration::ZERO;
    let mut layout_time = std::time::Duration::ZERO;
    const FRAMES: u32 = 200;
    for frame in 0..=FRAMES {
        model.boss_bars[0].progress = f64::from(frame % 100) / 100.0;
        let data = hud_data_source(&model);
        let started = Instant::now();
        let bound = bind_shared(&tree, &data, &library);
        let bind_elapsed = started.elapsed();
        let started = Instant::now();
        black_box(bind_stateful(&tree, &data, &library, &mut state));
        let stateful_elapsed = started.elapsed();
        let started = Instant::now();
        black_box(render_bound(
            bound,
            [480.0, 270.0],
            &env,
            &ViewState::default(),
        ));
        if frame > 0 {
            bind_time += bind_elapsed;
            stateful_time += stateful_elapsed;
            layout_time += started.elapsed();
        }
    }
    eprintln!(
        "FRAME_COST changing_hud: cold_resolve={:.3}ms bind={:.3}ms stateful_bind={:.3}ms layout_emit={:.3}ms total={:.3}ms",
        cold_resolve.as_secs_f64() * 1e3,
        (bind_time / FRAMES).as_secs_f64() * 1e3,
        (stateful_time / FRAMES).as_secs_f64() * 1e3,
        (layout_time / FRAMES).as_secs_f64() * 1e3,
        ((bind_time + layout_time) / FRAMES).as_secs_f64() * 1e3
    );
}
