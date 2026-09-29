//! Layout, nine-slice, and emit against synthetic trees and real vanilla templates.
//! The `.local` pack is gitignored, so tests that need it skip (not fail) when it is
//! absent; the synthetic tests always run and pin the deterministic layout maths.

use std::collections::BTreeMap;
use std::path::PathBuf;

use json_ui::{
    Context, Draw, LaidOut, LayoutEnv, Rect, ResolvedControl, TextMeasure, TextureMeta,
    TextureSource, emit, layout, nine_slice, parse_texture_meta, resolve,
};
use serde_json::{Value, json};

// --- test backends ----------------------------------------------------------

struct ZeroText;
impl TextMeasure for ZeroText {
    fn extent(&self, _text: &str) -> [f64; 2] {
        [0.0, 0.0]
    }
}

/// Fixed-width font stub so a label has a deterministic, non-zero natural size.
struct MonoText;
impl TextMeasure for MonoText {
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 10.0]
    }
}

struct NoTextures;
impl TextureSource for NoTextures {
    fn texture(&self, _path: &str) -> Option<TextureMeta> {
        None
    }
}

struct MapTextures(BTreeMap<String, TextureMeta>);
impl TextureSource for MapTextures {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
        self.0.get(path).copied()
    }
}

/// Reads real `textures/ui/<name>.json` sidecars from the pack root.
struct DirTextures(PathBuf);
impl TextureSource for DirTextures {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
        let file = self.0.join(format!("{path}.json"));
        let text = std::fs::read_to_string(file).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        parse_texture_meta(&value)
    }
}

// --- builders ---------------------------------------------------------------

fn ctrl(
    name: &str,
    control_type: Option<&str>,
    props: Value,
    children: Vec<ResolvedControl>,
) -> ResolvedControl {
    let properties: BTreeMap<String, Value> = match props {
        Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    ResolvedControl {
        name: name.to_owned(),
        control_type: control_type.map(str::to_owned),
        base: None,
        unresolved_base: None,
        properties,
        children,
        factory: None,
    }
}

fn child_named<'a>(root: &'a LaidOut<'a>, name: &str) -> &'a LaidOut<'a> {
    root.children
        .iter()
        .find(|child| child.control.name == name)
        .unwrap_or_else(|| panic!("missing child {name}"))
}

fn zero_env() -> LayoutEnv<'static> {
    LayoutEnv {
        text: &ZeroText,
        textures: &NoTextures,
    }
}

// --- anchors ----------------------------------------------------------------

/// A 10x10 child anchored `from == to` at each of the nine points lands on the
/// matching corner/edge/centre of a known 100x100 parent.
#[test]
fn nine_anchor_points_place_a_child() {
    let parent_size = json!([100, 100]);
    let cases = [
        ("top_left", [0.0, 0.0]),
        ("top_middle", [45.0, 0.0]),
        ("top_right", [90.0, 0.0]),
        ("left_middle", [0.0, 45.0]),
        ("center", [45.0, 45.0]),
        ("right_middle", [90.0, 45.0]),
        ("bottom_left", [0.0, 90.0]),
        ("bottom_middle", [45.0, 90.0]),
        ("bottom_right", [90.0, 90.0]),
    ];
    let env = zero_env();
    for (anchor, expected) in cases {
        let child = ctrl(
            "child",
            Some("panel"),
            json!({ "size": [10, 10], "anchor_from": anchor, "anchor_to": anchor }),
            vec![],
        );
        let root = ctrl(
            "root",
            Some("panel"),
            json!({ "size": parent_size, "anchor_from": "top_left", "anchor_to": "top_left" }),
            vec![child],
        );
        let placed = layout(&root, [100.0, 100.0], &env);
        let rect = child_named(&placed, "child").rect;
        assert_eq!([rect.x, rect.y], expected, "anchor {anchor}");
        assert_eq!([rect.w, rect.h], [10.0, 10.0], "anchor {anchor} size");
    }
}

/// `anchor_from` names the parent point, `anchor_to` the child point, as the
/// vanilla stack-progress arrows rely on: a child's top-left pinned to the
/// parent's bottom-right lands at the far corner.
#[test]
fn asymmetric_anchor_splits_parent_and_child_points() {
    let child = ctrl(
        "child",
        Some("panel"),
        json!({ "size": [10, 10], "anchor_from": "bottom_right", "anchor_to": "top_left" }),
        vec![],
    );
    let root = ctrl(
        "root",
        Some("panel"),
        json!({ "size": [100, 100], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![child],
    );
    let placed = layout(&root, [100.0, 100.0], &zero_env());
    let rect = child_named(&placed, "child").rect;
    assert_eq!([rect.x, rect.y], [100.0, 100.0]);
}

/// `offset` shifts a child after anchoring, in parent-relative pixels.
#[test]
fn offset_shifts_after_anchoring() {
    let child = ctrl(
        "child",
        Some("panel"),
        json!({ "size": [10, 10], "anchor_from": "top_left", "anchor_to": "top_left", "offset": [5, 7] }),
        vec![],
    );
    let root = ctrl(
        "root",
        Some("panel"),
        json!({ "size": [100, 100], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![child],
    );
    let placed = layout(&root, [100.0, 100.0], &zero_env());
    let rect = child_named(&placed, "child").rect;
    assert_eq!([rect.x, rect.y], [5.0, 7.0]);
}

// --- stack panels -----------------------------------------------------------

fn stack_root(orientation: &str, size: Value, children: Vec<ResolvedControl>) -> ResolvedControl {
    ctrl(
        "stack",
        Some("stack_panel"),
        json!({ "size": size, "orientation": orientation, "anchor_from": "top_left", "anchor_to": "top_left" }),
        children,
    )
}

fn item(name: &str, size: Value) -> ResolvedControl {
    ctrl(name, Some("panel"), json!({ "size": size }), vec![])
}

#[test]
fn vertical_stack_packs_children_end_to_end() {
    let root = stack_root(
        "vertical",
        json!([100, 100]),
        vec![
            item("a", json!(["100%", 20])),
            item("b", json!(["100%", 30])),
            item("c", json!(["100%", 10])),
        ],
    );
    let placed = layout(&root, [100.0, 100.0], &zero_env());
    let y = |name| child_named(&placed, name).rect.y;
    let h = |name| child_named(&placed, name).rect.h;
    assert_eq!((y("a"), h("a")), (0.0, 20.0));
    assert_eq!((y("b"), h("b")), (20.0, 30.0));
    assert_eq!((y("c"), h("c")), (50.0, 10.0));
}

#[test]
fn horizontal_stack_packs_along_x() {
    let root = stack_root(
        "horizontal",
        json!([100, 50]),
        vec![
            item("a", json!([20, "100%"])),
            item("b", json!([30, "100%"])),
        ],
    );
    let placed = layout(&root, [100.0, 50.0], &zero_env());
    assert_eq!(child_named(&placed, "a").rect.x, 0.0);
    assert_eq!(child_named(&placed, "b").rect.x, 20.0);
    assert_eq!(child_named(&placed, "b").rect.w, 30.0);
}

#[test]
fn fill_child_absorbs_leftover_main_axis() {
    let root = stack_root(
        "vertical",
        json!([100, 100]),
        vec![
            item("a", json!(["100%", 20])),
            item("b", json!(["100%", "fill"])),
            item("c", json!(["100%", 10])),
        ],
    );
    let placed = layout(&root, [100.0, 100.0], &zero_env());
    assert_eq!(child_named(&placed, "b").rect.h, 70.0);
    assert_eq!(child_named(&placed, "c").rect.y, 90.0);
}

/// `100%c` sizes a container to its children's content extent (a stack sums, a
/// plain panel takes the max).
#[test]
fn percent_children_measures_content_extent() {
    let panel = ctrl(
        "panel",
        Some("panel"),
        json!({ "size": ["100%c", "100%c"], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![item("a", json!([40, 15])), item("b", json!([25, 30]))],
    );
    let root = ctrl(
        "root",
        Some("panel"),
        json!({ "size": [200, 200], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![panel],
    );
    let placed = layout(&root, [200.0, 200.0], &zero_env());
    let rect = child_named(&placed, "panel").rect;
    assert_eq!([rect.w, rect.h], [40.0, 30.0]);
}

/// A label's `default` width comes from its measured text extent.
#[test]
fn label_default_width_is_text_extent() {
    let env = LayoutEnv {
        text: &MonoText,
        textures: &NoTextures,
    };
    let label = ctrl(
        "label",
        Some("label"),
        json!({ "size": ["default", 10], "text": "hello", "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![],
    );
    let root = ctrl(
        "root",
        Some("panel"),
        json!({ "size": [200, 50], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![label],
    );
    let placed = layout(&root, [200.0, 50.0], &env);
    assert_eq!(child_named(&placed, "label").rect.w, 30.0);
}

// --- nine-slice -------------------------------------------------------------

#[test]
fn nine_slice_splits_asymmetric_sidecar_into_nine_quads() {
    let meta =
        parse_texture_meta(&json!({ "nineslice_size": [8, 23, 8, 8], "base_size": [18, 33] }))
            .unwrap();
    let quads = nine_slice(Rect::new(0.0, 0.0, 225.0, 200.0), &meta);
    assert_eq!(quads.len(), 9);

    // Top-left corner: native size, source top-left of the texture.
    let top_left = quads[0];
    assert_eq!(
        (
            top_left.dest.x,
            top_left.dest.y,
            top_left.dest.w,
            top_left.dest.h
        ),
        (0.0, 0.0, 8.0, 23.0)
    );
    assert_eq!((top_left.uv.u0, top_left.uv.v0), (0.0, 0.0));
    assert!((top_left.uv.u1 - 8.0 / 18.0).abs() < 1e-6);
    assert!((top_left.uv.v1 - 23.0 / 33.0).abs() < 1e-6);

    // Centre: stretched on both axes, sampling the 2x2 middle of the source.
    let centre = quads[4];
    assert_eq!(
        (centre.dest.x, centre.dest.y, centre.dest.w, centre.dest.h),
        (8.0, 23.0, 225.0 - 16.0, 200.0 - 31.0)
    );

    // Bottom-right corner: native size again, pinned to the far corner.
    let bottom_right = quads[8];
    assert_eq!(
        (
            bottom_right.dest.x,
            bottom_right.dest.y,
            bottom_right.dest.w,
            bottom_right.dest.h
        ),
        (217.0, 192.0, 8.0, 8.0)
    );
}

#[test]
fn zero_top_and_bottom_leave_only_the_stretched_middle_row() {
    // [left, top, right, bottom] = [1, 0, 7, 0]: only the middle row survives.
    let meta =
        parse_texture_meta(&json!({ "nineslice_size": [1, 0, 7, 0], "base_size": [10, 10] }))
            .unwrap();
    let quads = nine_slice(Rect::new(0.0, 0.0, 100.0, 100.0), &meta);
    assert_eq!(quads.len(), 3);
    assert!(quads.iter().all(|q| q.dest.y == 0.0 && q.dest.h == 100.0));
    let widths: Vec<f64> = quads.iter().map(|q| q.dest.w).collect();
    assert_eq!(widths, vec![1.0, 92.0, 7.0]);
}

#[test]
fn nine_slice_image_emits_nine_sprites() {
    let mut map = BTreeMap::new();
    map.insert(
        "textures/ui/panel".to_owned(),
        TextureMeta {
            base_size: [16.0, 16.0],
            nineslice: Some(json_ui::NineSlice {
                left: 4.0,
                top: 4.0,
                right: 4.0,
                bottom: 4.0,
            }),
        },
    );
    let env = LayoutEnv {
        text: &ZeroText,
        textures: &MapTextures(map),
    };
    let image = ctrl(
        "bg",
        Some("image"),
        json!({ "texture": "textures/ui/panel", "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![],
    );
    let root = ctrl(
        "root",
        Some("panel"),
        json!({ "size": [80, 60], "anchor_from": "top_left", "anchor_to": "top_left" }),
        vec![image],
    );
    let placed = layout(&root, [80.0, 60.0], &env);
    let sprites = emit(&placed, &env)
        .iter()
        .filter(|node| matches!(node.draw, Draw::Sprite { .. }))
        .count();
    assert_eq!(sprites, 9);
}

// --- end to end -------------------------------------------------------------

fn pack_root() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/assets/bedrock-samples/v1.26.30.32-preview/full/resource_pack");
    dir.join("ui").is_dir().then_some(dir)
}

#[test]
fn main_panel_no_buttons_lays_out_with_nine_slice_background() {
    let Some(root) = pack_root() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let catalog = json_ui::Catalog::load_dir(&root.join("ui")).expect("index files load");
    let control = resolve(
        &catalog,
        "common_dialogs.main_panel_no_buttons",
        &Context::desktop(),
    )
    .control
    .expect("main_panel_no_buttons resolves");

    let env = LayoutEnv {
        text: &ZeroText,
        textures: &DirTextures(root),
    };
    let placed = layout(&control, [225.0, 200.0], &env);

    // Root fills the virtual dialog bounds.
    assert_eq!(
        (placed.rect.x, placed.rect.y, placed.rect.w, placed.rect.h),
        (0.0, 0.0, 225.0, 200.0)
    );

    // panel_indent inset from the arithmetic size and offset.
    let indent = child_named(&placed, "panel_indent");
    assert_eq!(
        (indent.rect.x, indent.rect.y, indent.rect.w, indent.rect.h),
        (8.0, 23.0, 209.0, 169.0)
    );

    // title_label is centred near the top and within the dialog.
    let title = child_named(&placed, "title_label");
    assert_eq!(title.rect.y, 9.0);
    assert!(title.rect.x >= 0.0 && title.rect.x <= 225.0);
    assert!(title.rect.w >= 0.0);

    // The background is nine-sliced: hollow_3 emits nine sprite quads, first drawn.
    let draws = emit(&placed, &env);
    let bg_sprites = draws
        .iter()
        .filter(|node| {
            matches!(&node.draw, Draw::Sprite { texture, .. }
                if texture == "textures/ui/dialog_background_hollow_3")
        })
        .count();
    assert_eq!(
        bg_sprites, 9,
        "hollow_3 background nine-slices into nine quads"
    );

    // Deterministic: a second run produces an identical draw list.
    let again = emit(&layout(&control, [225.0, 200.0], &env), &env);
    assert_eq!(draws, again);
}
