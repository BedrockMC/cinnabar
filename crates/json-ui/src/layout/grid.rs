//! Grid placement: column counts and the cells of fixed and rescaling grids.

use serde_json::Value;

use super::{LayoutEnv, Rect, ResolvedControl, place_by_anchor, resolve_size};

/// A `grid`'s column count: `grid_dimensions` (`[columns, rows]`) when given,
/// else `Some(None)` for a horizontally rescaling grid that fits its width.
pub(super) fn grid_columns(control: &ResolvedControl) -> Option<Option<usize>> {
    if control.control_type.as_deref() != Some("grid") {
        return None;
    }
    let fixed = control
        .properties
        .get("grid_dimensions")
        .and_then(Value::as_array)
        .and_then(|dims| dims.first()?.as_f64())
        .filter(|columns| *columns >= 1.0)
        .map(|columns| columns as usize)
        // A grid sized by a bound dimension is a one-column menu list.
        .or_else(|| {
            control
                .properties
                .contains_key("grid_dimension_binding")
                .then_some(1)
        });
    Some(fixed)
}

pub(super) fn fitted_columns(
    columns: Option<usize>,
    width: Option<f64>,
    pitch: f64,
    cells: usize,
) -> usize {
    columns
        .or_else(|| {
            let width = width?;
            (pitch > 0.0).then(|| (width / pitch).floor() as usize)
        })
        .unwrap_or(cells)
        .max(1)
}

/// Grid cells fill row-major from the top-left (or sit at their `grid_position`).
/// A fixed `grid_dimensions` grid listing its cells divides its own rect into
/// equal cells that size and place each child; otherwise cells pitch on the
/// largest child.
pub(super) fn grid_children<'a>(
    parent: &'a ResolvedControl,
    parent_rect: Rect,
    columns: Option<usize>,
    sibling_max: [f64; 2],
    env: &LayoutEnv,
) -> Vec<(&'a ResolvedControl, Rect)> {
    let rows = number_pair(parent, "grid_dimensions")
        .map(|[_, rows]| rows)
        .filter(|rows| *rows >= 1.0);
    // Template grids pitch on their template; listed cells share the grid's rect.
    let listed = !parent.properties.contains_key("grid_item_template");
    let cell = match (columns, rows) {
        (Some(columns), Some(rows)) if listed && parent_rect.w > 0.0 && parent_rect.h > 0.0 => {
            Some([parent_rect.w / columns as f64, parent_rect.h / rows])
        }
        _ => None,
    };
    let cell_rect = cell.map(|[w, h]| Rect::new(parent_rect.x, parent_rect.y, w, h));
    let sizes: Vec<[f64; 2]> = parent
        .children
        .iter()
        .map(|child| resolve_size(child, cell_rect.unwrap_or(parent_rect), sibling_max, env))
        .collect();
    let pitch = cell.unwrap_or_else(|| {
        sizes.iter().fold([0.0f64, 0.0f64], |acc, size| {
            [acc[0].max(size[0]), acc[1].max(size[1])]
        })
    });
    let columns = fitted_columns(columns, Some(parent_rect.w), pitch[0], sizes.len());
    parent
        .children
        .iter()
        .zip(sizes)
        .enumerate()
        .map(|(index, (child, size))| {
            let [column, row] = number_pair(child, "grid_position")
                .unwrap_or([(index % columns) as f64, (index / columns) as f64]);
            let (x, y) = (
                parent_rect.x + column * pitch[0],
                parent_rect.y + row * pitch[1],
            );
            let rect = cell.map_or(Rect::new(x, y, size[0], size[1]), |[w, h]| {
                place_by_anchor(child, Rect::new(x, y, w, h), size, env)
            });
            (child, rect)
        })
        .collect()
}

fn number_pair(control: &ResolvedControl, key: &str) -> Option<[f64; 2]> {
    let pair = control.properties.get(key)?.as_array()?;
    Some([pair.first()?.as_f64()?, pair.get(1)?.as_f64()?])
}
