//! Layout parity regressions against the client's layout rules, one per audited
//! row (`G`, `S`, `D` ids). Trees are written in pack syntax and laid out in a
//! 100x100 screen unless a test says otherwise.

use std::collections::BTreeMap;

use json_ui::{
    ControlLibrary, ControlRef, DataSource, LaidOut, LayoutEnv, ResolvedControl, Scalar,
    TextMeasure, TextureMeta, TextureSource, bind, layout,
};

use serde_json::{Value, json};

struct MonoText;

impl TextMeasure for MonoText {
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 10.0]
    }
}

/// Every texture is 40x20.
struct Textures;

impl TextureSource for Textures {
    fn texture(&self, _path: &str) -> Option<TextureMeta> {
        Some(TextureMeta {
            base_size: [40.0, 20.0],
            nineslice: None,
        })
    }
}

fn env() -> LayoutEnv<'static> {
    LayoutEnv {
        text: &MonoText,
        textures: &Textures,
    }
}

/// A control from pack syntax: `{"type": ..., "controls": [{"name": {...}}]}`.
fn control(name: &str, body: Value) -> ResolvedControl {
    let Value::Object(mut map) = body else {
        panic!("{name}: not an object");
    };
    let control_type = map
        .remove("type")
        .and_then(|kind| kind.as_str().map(str::to_owned));
    let children = match map.remove("controls") {
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| {
                let Value::Object(entry) = item else {
                    panic!("{name}: child not an object");
                };
                let (child, body) = entry.into_iter().next().expect("named child");
                control(&child, body)
            })
            .collect(),
        _ => Vec::new(),
    };
    ResolvedControl {
        name: name.to_owned(),
        control_type,
        base: None,
        unresolved_base: None,
        properties: map.into_iter().collect::<BTreeMap<_, _>>(),
        children,
        factory: None,
    }
}

/// `controls` under a 100x100 top-left root.
fn screen(controls: Value) -> ResolvedControl {
    control(
        "root",
        json!({
            "type": "panel",
            "size": [100, 100],
            "anchor_from": "top_left",
            "anchor_to": "top_left",
            "controls": controls,
        }),
    )
}

fn find<'a>(node: &'a LaidOut<'a>, name: &str) -> &'a LaidOut<'a> {
    if node.control.name == name {
        return node;
    }
    node.children
        .iter()
        .find_map(|child| {
            let found = find(child, name);
            (found.control.name == name).then_some(found)
        })
        .unwrap_or(node)
}

/// `[x, y, w, h]` of the control named `name`.
fn rect(root: &ResolvedControl, name: &str) -> [f64; 4] {
    let laid = layout(root, [100.0, 100.0], &env());
    let node = find(&laid, name);
    assert_eq!(node.control.name, name, "{name} not laid out");
    [node.rect.x, node.rect.y, node.rect.w, node.rect.h]
}

fn size(root: &ResolvedControl, name: &str) -> [f64; 2] {
    let [_, _, w, h] = rect(root, name);
    [w, h]
}

fn top_left(body: Value) -> Value {
    let mut body = body;
    body["anchor_from"] = json!("top_left");
    body["anchor_to"] = json!("top_left");
    body
}

// G03: upper-case units and a repeated sign parse as the client normalizes them.
#[test]
fn g03_normalized_expressions() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": ["20PX", 10] } },
        { "b": { "type": "panel", "size": ["100% + -4px", 10] } },
    ]));
    assert_eq!(size(&root, "a"), [20.0, 10.0]);
    assert_eq!(size(&root, "b"), [96.0, 10.0]);
}

// G05: an ordinary panel's `%c` sums its children.
#[test]
fn g05_percent_children_sums() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": ["100%c", 10],
        "controls": [
            { "a": { "type": "panel", "size": [20, 10] } },
            { "b": { "type": "panel", "size": [30, 10] } },
        ],
    } }]));
    assert_eq!(size(&root, "p"), [50.0, 10.0]);
}

// G06: `%cm` is the largest child whatever its coefficient.
#[test]
fn g06_child_max_ignores_its_coefficient() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": ["50%cm", 10],
        "controls": [{ "c": { "type": "panel", "size": [40, 10] } }],
    } }]));
    assert_eq!(size(&root, "p"), [40.0, 10.0]);
}

// G07: `%sm` reads a sibling's resolved size, coefficient ignored.
#[test]
fn g07_sibling_max_reads_resolved_siblings() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": [100, 20],
        "controls": [
            { "a": { "type": "panel", "size": ["50%", 10] } },
            { "b": { "type": "panel", "size": ["50%sm", 10] } },
        ],
    } }]));
    assert_eq!(size(&root, "b"), [50.0, 10.0]);
}

// G08: a height from the width reads the width after its bounds.
#[test]
fn g08_own_width_is_read_after_clamping() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": [100, "100%x"], "max_size": [50, 200],
    } }]));
    assert_eq!(size(&root, "p"), [50.0, 50.0]);
}

// G09: a width from the height resolves after the height.
#[test]
fn g09_own_height_resolves_first() {
    let root = screen(json!([{ "p": { "type": "panel", "size": ["100%y", 40] } }]));
    assert_eq!(size(&root, "p"), [40.0, 40.0]);
}

// G11: a `default` offset is no offset.
#[test]
fn g11_default_offset_is_none() {
    let root = screen(json!([{ "p": top_left(json!({
        "type": "panel", "size": [20, 10], "offset": ["default", 0],
    })) }]));
    assert_eq!(rect(&root, "p")[0], 0.0);
}

// G12: `default`/`fill` bounds install no rule.
#[test]
fn g12_keyword_bounds_are_unbounded() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": [20, 10], "min_size": ["fill", 0] } },
        { "b": { "type": "panel", "size": [20, 10], "min_size": ["default", 0] } },
        { "c": { "type": "panel", "size": [120, 10], "max_size": ["default", 100] } },
    ]));
    assert_eq!(size(&root, "a")[0], 20.0);
    assert_eq!(size(&root, "b")[0], 20.0);
    assert_eq!(size(&root, "c")[0], 120.0);
}

// G15: bounds read the control's own other axis, its siblings and its children.
#[test]
fn g15_bounds_read_their_full_context() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": [80, 1], "min_size": [0, "50%x"] } },
        { "b": { "type": "panel", "size": [80, 20], "max_size": ["50%y", 100] } },
        { "tall": { "type": "panel", "size": [10, 80] } },
        { "c": { "type": "panel", "size": [10, 20], "min_size": [0, "100%sm"] } },
        { "d": {
            "type": "stack_panel", "orientation": "vertical", "size": [20, 10],
            "min_size": [0, "100%cm"],
            "controls": [
                { "x": { "type": "panel", "size": [20, 20] } },
                { "y": { "type": "panel", "size": [20, 30] } },
            ],
        } },
    ]));
    assert_eq!(size(&root, "a")[1], 40.0);
    assert_eq!(size(&root, "b")[0], 10.0);
    assert_eq!(size(&root, "c")[1], 80.0);
    assert_eq!(size(&root, "d")[1], 30.0);
}

// A childless control's `%c` bound builds no rule, so a label keeps its text.
#[test]
fn a_childless_percent_children_bound_is_no_rule() {
    let root = screen(json!([{ "l": {
        "type": "label", "text": "hello", "size": ["default", 10], "max_size": ["100%c", 10],
    } }]));
    assert_eq!(size(&root, "l"), [30.0, 10.0]);
}

// When the minimum exceeds the maximum, an over-large value takes the maximum.
#[test]
fn an_overflowing_value_takes_the_maximum_over_a_larger_minimum() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": [90, 10], "min_size": [60, 0], "max_size": [40, 100] } },
        { "b": { "type": "panel", "size": [20, 10], "min_size": [60, 0], "max_size": [40, 100] } },
    ]));
    assert_eq!(size(&root, "a")[0], 40.0);
    assert_eq!(size(&root, "b")[0], 60.0);
}

// `fill` off a stack's main axis is an empty rule.
#[test]
fn fill_outside_a_stack_is_zero() {
    let root = screen(json!([{ "p": { "type": "panel", "size": ["fill", 10] } }]));
    assert_eq!(size(&root, "p"), [0.0, 10.0]);
}

// G16: offsets read own, children-max and sibling units.
#[test]
fn g16_offsets_read_their_full_context() {
    let root = screen(json!([
        { "a": top_left(json!({ "type": "panel", "size": [20, 10], "offset": ["50%x", 0] })) },
        { "b": top_left(json!({
            "type": "panel", "size": [50, 20], "offset": ["100%cm", 0],
            "controls": [{ "c": { "type": "panel", "size": [40, 10] } }],
        })) },
    ]));
    assert_eq!(rect(&root, "a")[0], 10.0);
    assert_eq!(rect(&root, "b")[0], 40.0);
}

// G17: an inheriting control takes its largest resolved sibling.
#[test]
fn g17_inherit_max_sibling_width_reads_resolved_siblings() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": [100, 20],
        "controls": [
            { "a": { "type": "panel", "size": ["50%", 10] } },
            { "b": { "type": "panel", "size": [20, 10], "inherit_max_sibling_width": true } },
        ],
    } }]));
    assert_eq!(size(&root, "b"), [50.0, 10.0]);
}

// G18: a ratio-scaled image derives its default axis from the other.
#[test]
fn g18_default_size_scales_to_ratio() {
    let root = screen(json!([
        { "a": { "type": "image", "texture": "t", "size": ["default", 10],
                 "default_size_scales_to_ratio": true } },
        { "b": { "type": "image", "texture": "t", "size": [80, "default"],
                 "default_size_scales_to_ratio": true } },
        { "c": { "type": "image", "texture": "t", "size": ["default", "default"],
                 "default_size_scales_to_ratio": true } },
        { "d": { "type": "image", "texture": "t", "size": ["default", 10] } },
    ]));
    assert_eq!(size(&root, "a"), [20.0, 10.0]);
    assert_eq!(size(&root, "b"), [80.0, 40.0]);
    assert_eq!(size(&root, "c"), [40.0, 20.0]);
    assert_eq!(size(&root, "d"), [100.0, 10.0]);
}

fn stack(orientation: &str, extra: Value, controls: Value) -> Value {
    let mut body = json!({
        "type": "stack_panel", "orientation": orientation, "size": [100, 80],
        "controls": controls,
    });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    top_left(body)
}

// S02: orientation `none` chains both axes.
#[test]
fn s02_orientation_none_chains_both_axes() {
    let root = screen(json!([{ "s": stack("none", json!({}), json!([
        { "a": { "type": "panel", "size": [20, 10] } },
        { "b": { "type": "panel", "size": [30, 10] } },
    ])) }]));
    assert_eq!(rect(&root, "a"), [0.0, 0.0, 20.0, 10.0]);
    assert_eq!(rect(&root, "b"), [20.0, 10.0, 30.0, 10.0]);
}

// S03: stack children obey their bounds, `fill` included.
#[test]
fn s03_stack_children_are_bounded() {
    let root = screen(
        json!([{ "s": stack("horizontal", json!({ "size": [100, 20] }), json!([
        { "a": { "type": "panel", "size": [80, 10], "max_size": [20, 20] } },
        { "b": { "type": "panel", "size": [10, 10] } },
        { "f": { "type": "panel", "size": ["fill", 10], "max_size": [30, 10] } },
    ])) }]),
    );
    assert_eq!(rect(&root, "a")[2], 20.0);
    assert_eq!(rect(&root, "b")[0], 20.0);
    assert_eq!(rect(&root, "f")[2], 30.0);
}

// S04: child anchors apply only with `use_child_anchors`.
#[test]
fn s04_use_child_anchors() {
    let child = json!([{ "c": {
        "type": "panel", "size": [20, 10], "anchor_from": "bottom_right", "anchor_to": "top_left",
    } }]);
    let off = screen(json!([{ "s": stack("vertical", json!({}), child.clone()) }]));
    let on =
        screen(json!([{ "s": stack("vertical", json!({ "use_child_anchors": true }), child) }]));
    assert_eq!(rect(&off, "c")[..2], [0.0, 0.0]);
    assert_eq!(rect(&on, "c")[..2], [100.0, 10.0]);
}

// Stack children ignore `offset`.
#[test]
fn stack_children_ignore_offset() {
    let root = screen(json!([{ "s": stack("vertical", json!({}), json!([
        { "a": { "type": "panel", "size": [20, 10], "offset": [5, 5] } },
    ])) }]));
    assert_eq!(rect(&root, "a")[..2], [0.0, 0.0]);
}

// S05: `use_priority` hides the lowest priority that overflows.
#[test]
fn s05_priority_hides_the_overflow() {
    let root = screen(json!([{ "s": stack("horizontal",
        json!({ "size": [80, 20], "use_priority": true }), json!([
        { "a": { "type": "panel", "size": [60, 20], "priority": 1 } },
        { "b": { "type": "panel", "size": [60, 20], "priority": 2 } },
    ])) }]));
    let laid = layout(&root, [100.0, 100.0], &env());
    assert!(find(&laid, "a").visible);
    assert!(!find(&laid, "b").visible);
}

// S06: a hidden stack child adds no main-axis space.
#[test]
fn s06_hidden_children_collapse() {
    let root = screen(json!([{ "s": stack("vertical", json!({}), json!([
        { "a": { "type": "panel", "size": [20, 10], "visible": false } },
        { "b": { "type": "panel", "size": [20, 10] } },
    ])) }]));
    assert_eq!(rect(&root, "b")[1], 0.0);
}

fn template_grid(extra: Value, cells: usize, template: Value) -> Value {
    let mut controls: Vec<Value> = (0..cells)
        .map(|index| json!({ "cell": { "type": "panel", "size": template["size"], "collection_index": index } }))
        .collect();
    let mut node = template;
    node["grid_template_node"] = json!(true);
    controls.push(json!({ "template": node }));
    let mut body = json!({
        "type": "grid", "grid_item_template": "t.cell", "controls": controls,
    });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    top_left(body)
}

fn cells(root: &ResolvedControl) -> Vec<[f64; 4]> {
    let laid = layout(root, [100.0, 100.0], &env());
    let grid = find(&laid, "g");
    grid.children
        .iter()
        .map(|cell| [cell.rect.x, cell.rect.y, cell.rect.w, cell.rect.h])
        .collect()
}

// D01: a templated grid places no cell past its capacity.
#[test]
fn d01_capacity_limits_cells() {
    let root = screen(json!([{ "g": template_grid(
        json!({ "size": [20, 20], "grid_dimensions": [1, 1] }), 2,
        json!({ "type": "panel", "size": [20, 10] })) }]));
    assert_eq!(cells(&root).len(), 1);
}

// D02: a templated grid measures dimensions × template.
#[test]
fn d02_template_geometry_sizes_the_grid() {
    let root = screen(json!([{ "g": template_grid(
        json!({ "size": ["100%c", "100%c"], "grid_dimensions": [2, 2] }), 0,
        json!({ "type": "panel", "size": [20, 10] })) }]));
    assert_eq!(size(&root, "g"), [40.0, 20.0]);
}

// D04: a horizontally rescaling grid fits whole columns, centring the leftover.
#[test]
fn d04_horizontal_rescaling() {
    let root = screen(json!([{ "g": template_grid(
        json!({ "size": [90, "default"], "grid_rescaling_type": "horizontal",
                "maximum_grid_items": 6 }), 6,
        json!({ "type": "panel", "size": [20, 10] })) }]));
    let placed = cells(&root);
    assert_eq!(size(&root, "g"), [90.0, 20.0]);
    assert_eq!(placed.len(), 6);
    // A bound capacity arrives as a whole float.
    let bound = screen(json!([{ "g": template_grid(
        json!({ "size": [90, "default"], "grid_rescaling_type": "horizontal",
                "#maximum_grid_items": 6.0 }), 6,
        json!({ "type": "panel", "size": [20, 10] })) }]));
    assert_eq!(size(&bound, "g"), [90.0, 20.0]);
    assert_eq!(placed[0], [5.0, 0.0, 20.0, 10.0]);
    assert_eq!(placed[4], [5.0, 10.0, 20.0, 10.0]);
}

// D05: a vertical fill direction makes one column of whole rows.
#[test]
fn d05_fill_direction() {
    let root = screen(json!([{ "g": template_grid(
        json!({ "size": [105, 85], "grid_fill_direction": "vertical" }), 8,
        json!({ "type": "panel", "size": ["100%", 15] })) }]));
    let placed = cells(&root);
    assert_eq!(placed.len(), 5);
    assert_eq!(placed[1][..2], [0.0, 17.0]);
}

// D08: listed cells sit at their `grid_position`, dividing the grid evenly.
#[test]
fn d08_listed_cells_use_grid_position() {
    let root = screen(json!([{ "g": top_left(json!({
        "type": "grid", "size": [40, 40], "grid_dimensions": [2, 2],
        "controls": [
            { "a": { "type": "panel", "grid_position": [1, 1] } },
            { "b": { "type": "panel", "size": [5, 5] } },
        ],
    })) }]));
    assert_eq!(rect(&root, "a"), [20.0, 20.0, 20.0, 20.0]);
    assert_eq!(rect(&root, "b"), [0.0, 0.0, 5.0, 5.0]);
}

/// A library holding `t.cell`, a 20x10 panel.
struct CellLibrary;

impl ControlLibrary for CellLibrary {
    fn resolve(&self, reference: &ControlRef) -> Option<ResolvedControl> {
        (reference.name == "cell")
            .then(|| control("cell", json!({ "type": "panel", "size": [20, 10] })))
    }
}

fn bound_grid(body: Value, data: &DataSource) -> ResolvedControl {
    bind(&control("g", body), data, &CellLibrary)
}

fn instances(grid: &ResolvedControl) -> usize {
    grid.children
        .iter()
        .filter(|child| child.properties.get("grid_template_node").is_none())
        .count()
}

fn items(count: usize) -> DataSource {
    let mut data = DataSource::new();
    data.set_collection(
        "items",
        (0..count)
            .map(|_| json_ui::CollectionItem::new("item"))
            .collect(),
    );
    data
}

// D02: the template is created and kept without a collection.
#[test]
fn d02_template_is_kept_without_a_collection() {
    let grid = bound_grid(
        json!({ "type": "grid", "grid_dimensions": [2, 2], "grid_item_template": "t.cell" }),
        &DataSource::new(),
    );
    assert_eq!(instances(&grid), 4);
    assert!(
        grid.children
            .iter()
            .any(|child| child.properties.get("grid_template_node").is_some())
    );
}

// D03: an unanswered dimension binding keeps zero dimensions.
#[test]
fn d03_unresolved_dimension_binding_creates_nothing() {
    let grid = bound_grid(
        json!({ "type": "grid", "grid_dimension_binding": "#missing",
                "grid_item_template": "t.cell", "collection_name": "items" }),
        &items(3),
    );
    assert_eq!(instances(&grid), 0);
}

// D06: a fixed grid holds columns × rows; a rescaling one `maximum_grid_items`.
#[test]
fn d06_capacity_follows_the_grid_mode() {
    let fixed = bound_grid(
        json!({ "type": "grid", "grid_dimensions": [2, 2], "grid_rescaling_type": "none",
                "maximum_grid_items": 1, "grid_item_template": "t.cell", "collection_name": "items" }),
        &items(6),
    );
    let rescaling = bound_grid(
        json!({ "type": "grid", "grid_dimensions": [2, 2], "grid_rescaling_type": "horizontal",
                "maximum_grid_items": 6, "grid_item_template": "t.cell", "collection_name": "items" }),
        &items(6),
    );
    let unset = bound_grid(
        json!({ "type": "grid", "grid_rescaling_type": "horizontal",
                "grid_item_template": "t.cell", "collection_name": "items" }),
        &items(3),
    );
    assert_eq!(instances(&fixed), 4);
    assert_eq!(instances(&rescaling), 6);
    assert_eq!(instances(&unset), 0);
}

// D07: a bound `#maximum_grid_items` sets the rescaling capacity.
#[test]
fn d07_bound_maximum_grid_items() {
    let mut data = items(3);
    data.set_global("#limit", Scalar::Num(0.0));
    let grid = bound_grid(
        json!({ "type": "grid", "grid_rescaling_type": "horizontal", "maximum_grid_items": 3,
                "grid_item_template": "t.cell", "collection_name": "items",
                "bindings": [{ "binding_type": "global", "binding_name": "#limit",
                               "binding_name_override": "#maximum_grid_items" }] }),
        &data,
    );
    assert_eq!(instances(&grid), 0);
}

// D07: vanilla's container grid binds its collection's size as the capacity.
#[test]
fn d07_collection_total_items_sets_the_capacity() {
    let grid = bound_grid(
        json!({ "type": "grid", "grid_rescaling_type": "horizontal",
                "grid_item_template": "t.cell", "collection_name": "items",
                "bindings": [{ "binding_type": "collection", "binding_collection_name": "items",
                               "binding_name": "#collection_total_items",
                               "binding_name_override": "#maximum_grid_items" }] }),
        &items(5),
    );
    assert_eq!(instances(&grid), 5);
}

// D10: the grid reports its capacity as `#grid_number_size`.
#[test]
fn d10_grid_number_size() {
    let grid = bound_grid(
        json!({ "type": "grid", "grid_dimensions": [2, 2], "grid_item_template": "t.cell",
                "collection_name": "items", "text": "#grid_number_size" }),
        &items(1),
    );
    assert_eq!(
        grid.properties.get("text").and_then(Value::as_f64),
        Some(4.0)
    );
}

// G05: a button's `%c` counts only the state child it shows at rest.
#[test]
fn g05_state_children_hidden_at_rest_add_nothing() {
    let state = |name: &str| json!({ name: { "type": "panel", "size": [85, 25] } });
    let root = screen(json!([{ "b": {
        "type": "button", "size": ["100%c + 2px", 25],
        "default_control": "default", "hover_control": "hover",
        "pressed_control": "pressed", "locked_control": "locked",
        "controls": [state("default"), state("hover"), state("pressed"), state("locked")],
    } }]));
    assert_eq!(size(&root, "b"), [87.0, 25.0]);
}
