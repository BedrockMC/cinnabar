//! Bell body and lip; the frame belongs to the block state.
//!
//! Box sizes come from the bell texture's unwrap and the vertical placement from the
//! block's collision bounds; swing animation is not drawn yet.

use super::{
    atlas::BlockEntityAtlas,
    mesh::{BoxSpec, Layer, MeshBuilder, WHITE, model_matrix},
};

pub(super) fn emit(builder: &mut MeshBuilder, atlas: &BlockEntityAtlas, block: [i32; 3]) {
    let Some(texture) = atlas.texture("textures/entity/bell/bell", [32.0, 32.0]) else {
        return;
    };
    let matrix = model_matrix(block, [0.5, 0.0, 0.5], 0.0);
    builder.cuboid(
        Layer::Solid,
        &texture,
        matrix,
        BoxSpec::new([-3.0, 6.0, -3.0], [6.0, 7.0, 6.0], [0.0, 0.0]),
        WHITE,
    );
    builder.cuboid(
        Layer::Solid,
        &texture,
        matrix,
        BoxSpec::new([-4.0, 4.0, -4.0], [8.0, 2.0, 8.0], [0.0, 13.0]),
        WHITE,
    );
}
