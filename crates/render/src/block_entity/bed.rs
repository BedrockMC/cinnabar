//! Beds: one 16x16x6 mattress slab per half plus two 3x3x3 legs at its outer end.
//!
//! Box sizes and UV origins are recovered from the bed texture's unwrap (head slab at the
//! origin, foot slab 16 rows down, four legs below); which leg texture sits on which corner
//! is not derivable from the pack and needs native measurement.

use bevy::math::Mat4;

use super::{
    atlas::BlockEntityAtlas,
    mesh::{BoxSpec, Layer, MeshBuilder, WHITE, model_matrix},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BedModel {
    /// Texture stem under `textures/entity/bed`, for example `red` or `silver`.
    pub color: &'static str,
    /// Whether this block is the pillow half.
    pub head: bool,
    /// Bedrock `direction` state: 0 head toward south, 1 west, 2 north, 3 east.
    pub direction: u8,
}

/// The texture stem for a bed block entity's `color` (dye id, white first).
#[must_use]
pub const fn bed_color(dye_id: i64) -> Option<&'static str> {
    Some(match dye_id {
        0 => "white",
        1 => "orange",
        2 => "magenta",
        3 => "light_blue",
        4 => "yellow",
        5 => "lime",
        6 => "pink",
        7 => "gray",
        8 => "silver",
        9 => "cyan",
        10 => "purple",
        11 => "blue",
        12 => "brown",
        13 => "green",
        14 => "red",
        15 => "black",
        _ => return None,
    })
}

/// Yaw that points the model's +Z (head end) along the `direction` state.
fn yaw_degrees(direction: u8) -> f32 {
    match direction % 4 {
        0 => 0.0,
        1 => 270.0,
        2 => 180.0,
        _ => 90.0,
    }
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    model: &BedModel,
) {
    let Some(texture) = atlas.texture(
        &format!("textures/entity/bed/{}", model.color),
        [64.0, 64.0],
    ) else {
        return;
    };
    let base = model_matrix(block, [0.5, 0.0, 0.5], yaw_degrees(model.direction));
    // The slab is authored 16 wide, 16 long, 6 thick; a quarter turn lays it flat with its
    // large -Z face on top and the texture's top edge toward +Z.
    let slab = base * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
    let (slab_uv, legs) = if model.head {
        (
            [0.0, 0.0],
            [([-8.0, 5.0], [0.0, 38.0]), ([5.0, 5.0], [12.0, 38.0])],
        )
    } else {
        (
            [0.0, 16.0],
            [([-8.0, -8.0], [0.0, 44.0]), ([5.0, -8.0], [12.0, 44.0])],
        )
    };
    builder.cuboid(
        Layer::Solid,
        &texture,
        slab,
        BoxSpec::new([-8.0, -8.0, -9.0], [16.0, 16.0, 6.0], slab_uv),
        WHITE,
    );
    for ([x, z], uv) in legs {
        builder.cuboid(
            Layer::Solid,
            &texture,
            base,
            BoxSpec::new([x, 0.0, z], [3.0, 3.0, 3.0], uv),
            WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;

    use super::*;

    #[test]
    fn colors_follow_dye_order_with_silver_for_light_gray() {
        assert_eq!(bed_color(0), Some("white"));
        assert_eq!(bed_color(8), Some("silver"));
        assert_eq!(bed_color(15), Some("black"));
        assert_eq!(bed_color(16), None);
    }

    #[test]
    fn yaw_points_the_head_end_along_the_direction_state() {
        for (direction, expected) in [
            (0, Vec3::Z),
            (1, Vec3::NEG_X),
            (2, Vec3::NEG_Z),
            (3, Vec3::X),
        ] {
            let head = Mat4::from_rotation_y(yaw_degrees(direction).to_radians())
                .transform_vector3(Vec3::Z);
            assert!(head.abs_diff_eq(expected, 1.0e-5), "{direction} {head:?}");
        }
    }

    #[test]
    fn the_slab_lies_flat_between_the_leg_tops_and_nine_pixels() {
        let slab = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let bottom = slab.transform_point3(Vec3::new(0.0, 0.0, -3.0));
        let top = slab.transform_point3(Vec3::new(0.0, 0.0, -9.0));
        assert!((bottom.y - 3.0).abs() < 1.0e-5 && (top.y - 9.0).abs() < 1.0e-5);
    }
}
