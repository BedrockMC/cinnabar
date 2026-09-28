//! Decorated pots: a body with four sherd faces, a neck and a lip.
//!
//! Neck, lip and top/bottom plane regions follow the pot base texture's unwrap and the side
//! quads use the shared side tile plus the sherd pattern; heights, wobble and which sherd
//! sits on which face need native measurement.

use bevy::math::Vec3;

use super::{
    atlas::{AtlasRect, BlockEntityAtlas},
    mesh::{BoxSpec, Facing, Layer, MeshBuilder, WHITE, model_matrix},
};

/// Sherd patterns sit this far off the side tile so they never fight it, in pixels.
const PATTERN_LIFT: f32 = 0.02;
const BODY_HALF: f32 = 7.0;
const BODY_HEIGHT: f32 = 16.0;

#[derive(Clone, Debug, PartialEq)]
pub struct DecoratedPotModel {
    pub facing: Facing,
    /// Pattern texture stems (for example `archer_pottery_pattern`) for back, left, right,
    /// front; `None` shows the plain side.
    pub sherds: [Option<String>; 4],
}

/// The pattern texture stem for a sherd item name; the plain brick shows no pattern.
#[must_use]
pub fn sherd_pattern(item: &str) -> Option<String> {
    let name = item.strip_prefix("minecraft:")?;
    let pattern = name.strip_suffix("_pottery_sherd")?;
    Some(format!("{pattern}_pottery_pattern"))
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    model: &DecoratedPotModel,
) {
    let matrix = model_matrix(block, [0.5, 0.0, 0.5], model.facing.yaw_degrees());
    if let Some(base) = atlas.texture("textures/blocks/decorated_pot_base", [32.0, 32.0]) {
        builder.cuboid(
            Layer::Solid,
            &base,
            matrix,
            BoxSpec::new([-4.0, BODY_HEIGHT, -4.0], [8.0, 3.0, 8.0], [0.0, 0.0]),
            WHITE,
        );
        builder.cuboid(
            Layer::Solid,
            &base,
            matrix,
            BoxSpec::new([-3.0, BODY_HEIGHT + 3.0, -3.0], [6.0, 1.0, 6.0], [0.0, 5.0]),
            WHITE,
        );
        let plane = |texels: [f32; 4], y: f32| {
            let uv = base.rect_uv(texels);
            let (near, far) = (-BODY_HALF, BODY_HALF);
            let corners = [
                [far, y, far],
                [near, y, far],
                [near, y, near],
                [far, y, near],
            ];
            let world =
                corners.map(|corner| matrix.transform_point3(Vec3::from_array(corner)).to_array());
            (world, uv)
        };
        for (texels, y) in [
            ([0.0, 13.0, 14.0, 14.0], BODY_HEIGHT),
            ([14.0, 13.0, 14.0, 14.0], 0.0),
        ] {
            let (corners, uv) = plane(texels, y);
            builder.quad(Layer::Solid, corners, uv, WHITE);
        }
    }
    let Some(side) = atlas.texture("textures/blocks/decorated_pot_side", [16.0, 16.0]) else {
        return;
    };
    // Faces as outward direction, top-left first when seen from outside; index into `sherds`.
    let faces: [([[f32; 3]; 4], [f32; 3], usize); 4] = [
        // Front (-Z).
        (
            [
                [BODY_HALF, BODY_HEIGHT, -BODY_HALF],
                [-BODY_HALF, BODY_HEIGHT, -BODY_HALF],
                [-BODY_HALF, 0.0, -BODY_HALF],
                [BODY_HALF, 0.0, -BODY_HALF],
            ],
            [0.0, 0.0, -PATTERN_LIFT],
            3,
        ),
        // Back (+Z).
        (
            [
                [-BODY_HALF, BODY_HEIGHT, BODY_HALF],
                [BODY_HALF, BODY_HEIGHT, BODY_HALF],
                [BODY_HALF, 0.0, BODY_HALF],
                [-BODY_HALF, 0.0, BODY_HALF],
            ],
            [0.0, 0.0, PATTERN_LIFT],
            0,
        ),
        // Left as seen from the front (+X).
        (
            [
                [BODY_HALF, BODY_HEIGHT, BODY_HALF],
                [BODY_HALF, BODY_HEIGHT, -BODY_HALF],
                [BODY_HALF, 0.0, -BODY_HALF],
                [BODY_HALF, 0.0, BODY_HALF],
            ],
            [PATTERN_LIFT, 0.0, 0.0],
            1,
        ),
        // Right as seen from the front (-X).
        (
            [
                [-BODY_HALF, BODY_HEIGHT, -BODY_HALF],
                [-BODY_HALF, BODY_HEIGHT, BODY_HALF],
                [-BODY_HALF, 0.0, BODY_HALF],
                [-BODY_HALF, 0.0, -BODY_HALF],
            ],
            [-PATTERN_LIFT, 0.0, 0.0],
            2,
        ),
    ];
    for (corners, lift, index) in faces {
        let world = |offset: [f32; 3]| {
            corners.map(|corner| {
                matrix
                    .transform_point3(Vec3::from_array(corner) + Vec3::from_array(offset))
                    .to_array()
            })
        };
        builder.textured_quad(Layer::Solid, world([0.0; 3]), side.rect, WHITE);
        let pattern = model.sherds[index]
            .as_deref()
            .and_then(|stem| atlas.texture(&format!("textures/blocks/{stem}"), [16.0, 16.0]));
        if let Some(pattern) = pattern {
            let rect: AtlasRect = pattern.rect;
            builder.textured_quad(Layer::Solid, world(lift), rect, WHITE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sherd_items_map_to_pattern_textures_and_bricks_show_none() {
        assert_eq!(
            sherd_pattern("minecraft:archer_pottery_sherd").as_deref(),
            Some("archer_pottery_pattern")
        );
        assert_eq!(sherd_pattern("minecraft:brick"), None);
        assert_eq!(sherd_pattern("archer_pottery_sherd"), None);
    }
}
