//! Incremental refreshes must produce exactly what rebuilding every control does.

#[path = "support/java_pack.rs"]
mod java_pack;
use crate::support;

use std::sync::Arc;

use json_ui::{
    BindState, BossBar, CachedLibrary, Catalog, CatalogLibrary, CollectionItem, Context,
    DataSource, FactoryItem, FormRender, HUD_SCREEN, HudModel, HudSlot, HudTitle, LayoutEnv,
    MeasureCache, ResolveCache, ResolvedControl, Scalar, Sidebar, TextMeasure, TextureMeta,
    TextureSource, Timed, ViewState, bind_incremental, bind_stateful, hud_context, hud_data_source,
    rebind, render_bound, render_bound_cached, resolve,
};
use serde_json::Value;

struct FixedText;

impl TextMeasure for FixedText {
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 9.0]
    }
}

struct FixedTextures;

impl TextureSource for FixedTextures {
    fn texture(&self, _: &str) -> Option<TextureMeta> {
        Some(TextureMeta {
            base_size: [16.0; 2],
            pixels: [16.0; 2],
            nineslice: None,
        })
    }
}

const ROOT: [f64; 2] = [480.0, 270.0];

/// One screen refreshed both ways: every control rebuilt, and incrementally.
struct Pair<'c> {
    tree: Arc<ResolvedControl>,
    catalog: &'c Catalog,
    context: Context,
    full: BindState,
    incremental: (BindState, ResolveCache, MeasureCache),
    laid: Option<FormRender>,
}

impl<'c> Pair<'c> {
    fn new(catalog: &'c Catalog, reference: &str, context: Context) -> Self {
        let tree = Arc::new(resolve(catalog, reference, &context).control.unwrap());
        Self {
            tree,
            catalog,
            context,
            full: Default::default(),
            incremental: Default::default(),
            laid: None,
        }
    }

    /// Refresh both and assert the trees and their renders agree; returns what
    /// the incremental refresh rebuilt and placed.
    fn refresh(&mut self, data: DataSource, step: &str) -> Touched {
        let env = LayoutEnv {
            text: &FixedText,
            textures: &FixedTextures,
        };
        let library = |cache| CachedLibrary {
            library: CatalogLibrary {
                catalog: self.catalog,
                context: &self.context,
            },
            cache,
        };
        // The reference resolves every factory control afresh.
        let catalog_library = CatalogLibrary {
            catalog: self.catalog,
            context: &self.context,
        };
        let (expected, _) = bind_stateful(&self.tree, &data, &catalog_library, &mut self.full);
        let data = Arc::new(data);
        let (state, cache, measures) = &mut self.incremental;
        let bound = match self.laid.take() {
            Some(laid) => {
                let mut bound = laid.bound;
                rebind(
                    &self.tree,
                    &data,
                    &library(cache),
                    state,
                    &mut bound,
                    measures,
                );
                bound
            }
            None => bind_incremental(&self.tree, &data, &library(cache), state),
        };
        assert!(bound == expected, "{step}: bound trees differ");
        let render = render_bound_cached(bound, ROOT, &env, &ViewState::default(), measures);
        let cold = render_bound(expected, ROOT, &env, &ViewState::default());
        assert_eq!(render.nodes, cold.nodes, "{step}: draws differ");
        assert_eq!(render.hits, cold.hits, "{step}: hit regions differ");
        assert_eq!(render.report, cold.report, "{step}: reports differ");
        assert_eq!(
            render.cancel_target, cold.cancel_target,
            "{step}: cancel targets differ"
        );
        assert_eq!(
            render.root_panel, cold.root_panel,
            "{step}: root panels differ"
        );
        self.laid = Some(render);
        Touched {
            rebuilt: state.rebuilt_paths(),
            placed: measures.placed(),
        }
    }
}

/// What one incremental refresh redid.
struct Touched {
    /// Name paths of the controls the bind rebuilt.
    rebuilt: Vec<String>,
    /// Controls the layout placed rather than spliced.
    placed: usize,
}

/// Deterministic xorshift steps.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
}

fn vanilla_hud() -> Option<Catalog> {
    let vanilla = support::vanilla_pack().join("ui");
    if !vanilla.is_dir() {
        return None;
    }
    let mut catalog = Catalog::load_dir(&vanilla).unwrap();
    let java = java_pack::files();
    catalog.apply_pack(
        java.iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    Some(catalog)
}

fn lobby() -> HudModel {
    HudModel {
        survival_ui: true,
        hotbar_visible: true,
        xp_bar: true,
        level: 3,
        exp_progress: 0.4,
        chat_visible: true,
        chat_lifetime: 10.0,
        chat_background_opacity: 0.5,
        hotbar: (0..9)
            .map(|index| HudSlot {
                icon: Some(index),
                count: 1,
                selected: index == 0,
                durability: None,
            })
            .collect(),
        sidebar: Some(Sidebar {
            title: "Lobby".into(),
            rows: (0..15)
                .map(|row| (format!("Stat {row}"), (row * 3).to_string()))
                .collect(),
            background_opacity: 0.3,
            title_background_opacity: 0.4,
        }),
        ..HudModel::default()
    }
}

/// One random gameplay change to `model` at `now` seconds.
fn mutate(model: &mut HudModel, rng: &mut Rng, now: f64) -> &'static str {
    match rng.below(14) {
        0..=3 => {
            model.chat.push(Timed {
                text: format!("§7[Member] §fPlayer{}: hi {now}", rng.below(40)),
                born: now,
            });
            model.chat.retain(|line| line.born > now - 10.0);
            let excess = model.chat.len().saturating_sub(50);
            model.chat.drain(..excess);
            "chat line"
        }
        4 | 5 => {
            let sidebar = model.sidebar.get_or_insert_with(Sidebar::default);
            match rng.below(4) {
                0 if !sidebar.rows.is_empty() => {
                    sidebar.rows.remove(rng.below(sidebar.rows.len()));
                    "score row removed"
                }
                1 => {
                    sidebar.rows.push((format!("New {now}"), "0".into()));
                    "score row added"
                }
                _ if !sidebar.rows.is_empty() => {
                    let row = rng.below(sidebar.rows.len());
                    sidebar.rows[row].1 = rng.below(1000).to_string();
                    "score row changed"
                }
                _ => "score unchanged",
            }
        }
        6 => {
            model.title = rng.chance(70).then(|| HudTitle {
                title: format!("§6Round {}", rng.below(9)),
                subtitle: if rng.chance(50) {
                    "go".into()
                } else {
                    String::new()
                },
                fade_in: 0.5,
                stay: 3.5,
                fade_out: 1.0,
                background_alpha: 0.0,
                born: now,
            });
            "title"
        }
        7 => {
            model.actionbar = rng.chance(80).then(|| Timed {
                text: format!("Online: {} | Ping: {}ms", rng.below(300), rng.below(90)),
                born: now,
            });
            "actionbar"
        }
        8 => {
            match rng.below(3) {
                0 if model.boss_bars.len() < 3 => model.boss_bars.push(BossBar {
                    name: format!("Boss {now}"),
                    progress: 0.5,
                    color: "#aa00aa".into(),
                    notches: 6,
                }),
                1 if !model.boss_bars.is_empty() => {
                    model.boss_bars.pop();
                }
                _ => {
                    if let Some(bar) = model.boss_bars.first_mut() {
                        bar.progress = rng.below(100) as f64 / 100.0;
                    }
                }
            }
            "boss bars"
        }
        9 => {
            let slot = rng.below(9);
            let selected = rng.below(9);
            for (index, item) in model.hotbar.iter_mut().enumerate() {
                item.selected = index == selected;
            }
            model.hotbar[slot].count = rng.below(64) as u32;
            model.hotbar[slot].icon = (model.hotbar[slot].count > 0).then_some(slot);
            "hotbar"
        }
        10 => {
            match rng.below(5) {
                0 => model.chat_visible = !model.chat_visible,
                1 => model.hotbar_visible = !model.hotbar_visible,
                2 => model.survival_ui = !model.survival_ui,
                3 => model.sidebar = model.sidebar.take().xor(Some(lobby().sidebar.unwrap())),
                _ => model.paper_doll = !model.paper_doll,
            }
            "visibility"
        }
        11 => {
            model.level = rng.below(40) as u32;
            model.exp_progress = rng.below(100) as f64 / 100.0;
            model.xp_bar = rng.chance(90);
            "experience"
        }
        12 => {
            model.item_name = rng.chance(60).then(|| Timed {
                text: format!("Sword {}", rng.below(5)),
                born: now,
            });
            model.player_position = rng
                .chance(30)
                .then(|| format!("Position: {}, 64, 0", rng.below(500)));
            "item name"
        }
        _ => "unchanged",
    }
}

#[test]
fn random_hud_refreshes_match_full_rebinds() {
    let Some(catalog) = vanilla_hud() else {
        return;
    };
    let mut pair = Pair::new(&catalog, HUD_SCREEN, hud_context(&Context::desktop()));
    let mut model = lobby();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    pair.refresh(hud_data_source(&model), "initial");
    for step in 0..400 {
        let now = 1.0 + step as f64 * 0.25;
        let kind = mutate(&mut model, &mut rng, now);
        pair.refresh(hud_data_source(&model), &format!("step {step}: {kind}"));
    }
}

/// A chat line rebuilds only the chat, a score only the sidebar, a title only
/// its factory, once the screen has settled.
#[test]
fn hud_changes_rebuild_only_their_subtree() {
    let Some(catalog) = vanilla_hud() else {
        return;
    };
    let mut pair = Pair::new(&catalog, HUD_SCREEN, hud_context(&Context::desktop()));
    let mut model = lobby();
    for settle in 0..2 {
        model.chat.push(Timed {
            text: format!("settle {settle}"),
            born: 0.0,
        });
        pair.refresh(hud_data_source(&model), "settle");
    }
    // A control the last refresh created settles on the next.
    pair.refresh(hud_data_source(&model), "settle");
    let idle = pair.refresh(hud_data_source(&model), "idle");
    assert!(
        idle.rebuilt.is_empty(),
        "an unchanged refresh rebuilt {:?}",
        idle.rebuilt
    );
    let total = Pair::new(&catalog, HUD_SCREEN, hud_context(&Context::desktop()))
        .refresh(hud_data_source(&model), "cold")
        .placed;
    let changes: [(&str, &str, fn(&mut HudModel)); 3] = [
        ("chat", "chat", |model| {
            model.chat.push(Timed {
                text: "new line".into(),
                born: 1.0,
            });
        }),
        ("score", "sidebar", |model| {
            model.sidebar.as_mut().unwrap().rows[4].1 = "999".into();
        }),
        ("title", "hud_title_text", |model| {
            model.title = Some(HudTitle {
                title: "Round 2".into(),
                fade_in: 0.5,
                stay: 3.0,
                fade_out: 1.0,
                born: 2.0,
                ..HudTitle::default()
            });
        }),
    ];
    for (name, subtree, change) in changes {
        change(&mut model);
        let touched = pair.refresh(hud_data_source(&model), name);
        let paths = &touched.rebuilt;
        assert!(!paths.is_empty(), "{name}: nothing rebuilt");
        assert!(
            touched.placed * 4 < total,
            "{name}: placed {} of {total} controls",
            touched.placed
        );
        let outside: Vec<_> = paths
            .iter()
            .filter(|path| !path.contains(subtree) && !ancestor_of_subtree(paths, path, subtree))
            .collect();
        assert!(
            outside.is_empty(),
            "{name} rebuilt outside {subtree}: {outside:?}"
        );
        // The next refresh settles what the change created, then nothing rebuilds.
        pair.refresh(hud_data_source(&model), name);
        let idle = pair.refresh(hud_data_source(&model), name);
        assert!(
            idle.rebuilt.is_empty(),
            "{name}: settled refresh rebuilt {:?}",
            idle.rebuilt
        );
        assert_eq!(idle.placed, 0, "{name}: an unchanged tree placed controls");
    }
}

/// Whether `path` lies on the way down to a rebuilt control inside `subtree`.
fn ancestor_of_subtree(paths: &[String], path: &str, subtree: &str) -> bool {
    paths
        .iter()
        .any(|other| other.contains(subtree) && other.starts_with(&format!("{path}/")))
}

const SYNTHETIC: &str = r##"{
  "namespace": "inc",
  "row": {
    "type": "label", "text": "#row_text", "size": ["default", 10],
    "property_bag": { "#row_text": "seed" },
    "bindings": [
      { "binding_type": "collection", "binding_collection_name": "rows", "binding_name": "#row_text" },
      { "binding_type": "collection", "binding_collection_name": "rows", "binding_name": "#row_on", "binding_name_override": "#visible" }
    ]
  },
  "line": {
    "type": "label", "text": "#text", "size": ["default", 9],
    "property_bag": { "#text": "template" },
    "bindings": [
      { "binding_type": "collection", "binding_collection_name": "lines", "binding_name": "#line_tag", "binding_condition": "once" }
    ]
  },
  "cell": {
    "type": "image", "texture": "#cell_texture", "size": [16, 16],
    "bindings": [
      { "binding_type": "collection", "binding_collection_name": "cells", "binding_name": "#cell_texture" },
      { "binding_type": "collection_details", "binding_collection_name": "cells" }
    ]
  },
  "root": {
    "type": "panel", "size": ["100%", "100%"],
    "controls": [
      { "title": {
          "type": "label", "text": "#title", "anchor_from": "top_middle", "anchor_to": "top_middle",
          "size": ["default", 10],
          "bindings": [
            { "binding_name": "#title" },
            { "binding_name": "#title_on", "binding_name_override": "#visible" }
          ]
      } },
      { "mirror": {
          "type": "stack_panel", "size": ["100%c", "100%c"], "anchor_from": "top_left", "anchor_to": "top_left",
          "bindings": [
            { "binding_type": "view", "source_control_name": "title", "source_property_name": "#title_on", "target_property_name": "#visible" }
          ],
          "controls": [
            { "mirror_text": {
                "type": "label", "text": "#copy", "size": ["default", 10],
                "bindings": [
                  { "binding_type": "view", "source_control_name": "title", "source_property_name": "(#title + '!')", "target_property_name": "#copy" }
                ]
            } },
            { "counter": {
                "type": "label", "text": "#count", "size": ["default", 10],
                "bindings": [ { "binding_name": "#count", "binding_condition": "visible" } ]
            } }
          ]
      } },
      { "rows": {
          "type": "stack_panel", "orientation": "vertical", "size": ["100%cm", "100%c"],
          "anchor_from": "left_middle", "anchor_to": "left_middle",
          "collection_name": "rows",
          "factory": { "name": "rows_factory", "control_name": "inc.row" },
          "bindings": [ { "binding_name": "#row_count", "binding_name_override": "#collection_length" } ]
      } },
      { "lines": {
          "type": "stack_panel", "orientation": "vertical", "size": ["100%cm", "100%c"],
          "anchor_from": "bottom_left", "anchor_to": "bottom_left",
          "factory": { "name": "line_factory", "control_ids": { "line": "line@inc.line" } }
      } },
      { "cells": {
          "type": "grid", "size": ["100%c", "100%c"], "anchor_from": "right_middle", "anchor_to": "right_middle",
          "collection_name": "cells", "grid_item_template": "inc.cell",
          "grid_dimension_binding": "#cell_dims",
          "bindings": [ { "binding_name": "#cell_dims" } ]
      } },
      { "late": {
          "type": "panel", "size": [100, 20], "anchor_from": "bottom_right", "anchor_to": "bottom_right",
          "visible": false,
          "bindings": [ { "binding_name": "#late_on", "binding_name_override": "#visible", "binding_condition": "visibility_changed" } ],
          "controls": [
            { "late_text": {
                "type": "label", "text": "#late_text", "size": ["default", 10],
                "bindings": [ { "binding_name": "#late_text", "binding_condition": "once" } ]
            } }
          ]
      } },
      { "menu": {
          "type": "panel", "size": [60, 40], "anchor_from": "top_right", "anchor_to": "top_right",
          "focus_container": true, "collection_name": "cells",
          "controls": [
            { "back": {
                "type": "button", "size": [20, 10],
                "button_mappings": [
                  { "from_button_id": "button.menu_cancel", "to_button_id": "button.menu_exit", "mapping_type": "global" }
                ],
                "bindings": [ { "binding_name": "#gate_on", "binding_name_override": "#visible" } ]
            } },
            { "slot": {
                "type": "custom", "renderer": "inventory_item_renderer", "size": [16, 16],
                "collection_index": 0, "offset": [0, 20]
            } },
            { "root_panel": {
                "type": "panel", "size": [10, 10],
                "bindings": [ { "binding_name": "#title_on", "binding_name_override": "#visible" } ]
            } }
          ]
      } },
      { "gate": {
          "type": "panel", "size": [50, 20],
          "bindings": [ { "binding_name": "#gate_on", "binding_name_override": "#visible" } ],
          "controls": [
            { "gated": { "type": "label", "text": "#gate_text", "size": ["default", 10],
                "bindings": [ { "binding_name": "#gate_text" } ] } }
          ]
      } }
    ]
  }
}"##;

fn synthetic() -> Catalog {
    Catalog::from_files([
        ("ui/_global_variables.json", b"{}".as_slice()),
        (
            "ui/_ui_defs.json",
            br#"{"ui_defs":["ui/inc.json"]}"#.as_slice(),
        ),
        ("ui/inc.json", SYNTHETIC.as_bytes()),
    ])
    .unwrap()
}

/// The synthetic screen's controller state.
#[derive(Clone, Default)]
struct Synthetic {
    title: Option<String>,
    count: i64,
    rows: Vec<(String, bool)>,
    lines: Vec<(String, f64)>,
    cells: Vec<String>,
    late: Option<String>,
    gate: Option<String>,
}

impl Synthetic {
    fn data(&self) -> DataSource {
        let mut data = DataSource::new();
        data.set_strict(true);
        data.set_global("#title_on", Scalar::Bool(self.title.is_some()));
        if let Some(title) = &self.title {
            data.set_global("#title", Scalar::Text(title.clone()));
        }
        data.set_global("#count", Scalar::Int(self.count));
        data.set_global("#row_count", Scalar::Int(self.rows.len() as i64));
        data.set_collection(
            "rows",
            self.rows
                .iter()
                .map(|(text, on)| {
                    CollectionItem::default()
                        .with("#row_text", Scalar::Text(text.clone()))
                        .with("#row_on", Scalar::Bool(*on))
                })
                .collect(),
        );
        data.set_collection(
            "lines",
            self.lines
                .iter()
                .map(|(text, _)| {
                    CollectionItem::default().with("#line_tag", Scalar::Text(text.clone()))
                })
                .collect(),
        );
        data.set_factory(
            "line_factory",
            self.lines
                .iter()
                .enumerate()
                .map(|(index, (text, born))| {
                    FactoryItem::new("line", *born)
                        .at("lines", index)
                        .value("#text", Scalar::Text(text.clone()))
                })
                .collect(),
        );
        data.set_grid_dimensions("#cell_dims", [2, self.cells.len().div_ceil(2) as u32]);
        data.set_collection(
            "cells",
            self.cells
                .iter()
                .map(|texture| {
                    CollectionItem::default().with("#cell_texture", Scalar::Text(texture.clone()))
                })
                .collect(),
        );
        data.set_global("#late_on", Scalar::Bool(self.late.is_some()));
        if let Some(late) = &self.late {
            data.set_global("#late_text", Scalar::Text(late.clone()));
        }
        data.set_global("#gate_on", Scalar::Bool(self.gate.is_some()));
        if let Some(gate) = &self.gate {
            data.set_global("#gate_text", Scalar::Text(gate.clone()));
        }
        data
    }

    fn mutate(&mut self, rng: &mut Rng, now: f64) -> &'static str {
        match rng.below(9) {
            0 => {
                self.title = rng.chance(70).then(|| format!("title {}", rng.below(5)));
                "title"
            }
            1 => {
                self.count = rng.below(4) as i64;
                "count"
            }
            2 if !self.rows.is_empty() && rng.chance(40) => {
                self.rows.remove(rng.below(self.rows.len()));
                "row removed"
            }
            2 => {
                let at = rng.below(self.rows.len() + 1);
                self.rows.insert(at, (format!("row {now}"), rng.chance(80)));
                "row added"
            }
            3 if !self.rows.is_empty() => {
                let row = rng.below(self.rows.len());
                self.rows[row] = (format!("edit {}", rng.below(9)), rng.chance(70));
                "row changed"
            }
            4 => {
                self.lines.push((format!("line {now}"), now));
                if self.lines.len() > 6 || rng.chance(20) {
                    self.lines.remove(0);
                }
                "line"
            }
            5 => {
                if rng.chance(50) && !self.cells.is_empty() {
                    self.cells.pop();
                } else {
                    self.cells.push(format!("textures/cell{}", rng.below(4)));
                }
                "cells"
            }
            6 => {
                self.late = rng.chance(60).then(|| format!("late {}", rng.below(5)));
                "late"
            }
            7 => {
                self.gate = rng.chance(60).then(|| format!("gate {}", rng.below(5)));
                "gate"
            }
            _ => "unchanged",
        }
    }
}

#[test]
fn random_synthetic_refreshes_match_full_rebinds() {
    let catalog = synthetic();
    for seed in 1..=8u64 {
        let mut pair = Pair::new(&catalog, "inc.root", Context::desktop());
        let mut screen = Synthetic::default();
        let mut rng = Rng(seed.wrapping_mul(0x2545_f491_4f6c_dd1d));
        pair.refresh(screen.data(), "initial");
        for step in 0..150 {
            let kind = screen.mutate(&mut rng, step as f64);
            pair.refresh(screen.data(), &format!("seed {seed} step {step}: {kind}"));
        }
    }
}

/// Adding and removing collection rows and factory lines rebuilds only that
/// list, and still matches a full rebind.
#[test]
fn collection_changes_rebuild_only_their_list() {
    let catalog = synthetic();
    let mut pair = Pair::new(&catalog, "inc.root", Context::desktop());
    let mut screen = Synthetic {
        title: Some("t".into()),
        rows: (0..4).map(|row| (format!("row {row}"), true)).collect(),
        lines: (0..3).map(|line| (format!("line {line}"), 0.0)).collect(),
        cells: vec!["textures/a".into(); 3],
        ..Synthetic::default()
    };
    for _ in 0..3 {
        pair.refresh(screen.data(), "settle");
    }
    let cases: [(&str, &str, fn(&mut Synthetic)); 5] = [
        ("row added", "/rows", |screen| {
            screen.rows.push(("row 4".into(), true))
        }),
        ("row removed", "/rows", |screen| {
            screen.rows.remove(1);
        }),
        ("line added", "/line", |screen| {
            screen.lines.push(("line 3".into(), 1.0))
        }),
        ("line dropped", "/line", |screen| {
            screen.lines.remove(0);
        }),
        // The menu panel lists the same collection.
        ("cell added", "/cells|/menu", |screen| {
            screen.cells.push("textures/b".into())
        }),
    ];
    for (name, list, change) in cases {
        change(&mut screen);
        let paths = pair.refresh(screen.data(), name).rebuilt;
        assert!(!paths.is_empty(), "{name}: nothing rebuilt");
        for path in &paths {
            assert!(
                list.split('|')
                    .any(|list| { path.contains(list) || ancestor_of_subtree(&paths, path, list) }),
                "{name} rebuilt {path} outside {list}: {paths:?}"
            );
        }
        for _ in 0..2 {
            pair.refresh(screen.data(), name);
        }
        let idle = pair.refresh(screen.data(), name);
        assert!(
            idle.rebuilt.is_empty(),
            "{name}: settled refresh rebuilt {:?}",
            idle.rebuilt
        );
    }
}

#[test]
fn views_follow_rebuilt_sources_into_reused_controls() {
    let catalog = synthetic();
    let mut pair = Pair::new(&catalog, "inc.root", Context::desktop());
    let mut screen = Synthetic {
        title: Some("a".into()),
        ..Synthetic::default()
    };
    for title in [
        Some("a"),
        Some("a"),
        Some("b"),
        None,
        None,
        Some("c"),
        Some("c"),
    ] {
        screen.title = title.map(str::to_owned);
        pair.refresh(screen.data(), "title");
    }
    let bound = &pair.laid.as_ref().unwrap().bound;
    let copy = find(bound, "mirror_text").properties.get("#copy").cloned();
    assert_eq!(copy, Some(Value::from("c!")));
}

fn find<'a>(control: &'a ResolvedControl, name: &str) -> &'a ResolvedControl {
    fn walk<'a>(control: &'a ResolvedControl, name: &str) -> Option<&'a ResolvedControl> {
        if control.name == name {
            return Some(control);
        }
        control.children.iter().find_map(|child| walk(child, name))
    }
    walk(control, name).unwrap_or_else(|| panic!("no control {name}"))
}
