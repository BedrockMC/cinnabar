use crate::{self as render_api, shader_source};
use assets::{EntityRenderMaterial, EntityRenderMaterialState};
use bevy::{prelude::Msaa, render::render_resource::Specializer, shader::ShaderDefVal};

use super::super::{
    ActorPipelineKey, ActorPipelineSpecializer, actor_bind_group_layout, actor_pipeline_descriptor,
};

#[path = "../../tests/it/support/actor_raster.rs"]
mod actor_raster;
#[path = "../../tests/it/support/gpu_snapshot.rs"]
mod gpu_snapshot;

#[test]
fn actor_material_black_plate_blends_encoded_destination_channels() {
    let Some(gpu) = gpu_snapshot::Gpu::for_fixture(
        "actor_material_black_plate_blends_encoded_destination_channels",
    ) else {
        return;
    };
    let material = crate::ActorMaterial {
        state: Some(EntityRenderMaterialState {
            alpha_test: true,
            cull: false,
            blend: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let mut descriptor = actor_pipeline_descriptor(actor_bind_group_layout());
    ActorPipelineSpecializer
        .specialize(
            ActorPipelineKey {
                msaa: Msaa::Off,
                hdr: false,
                enhanced: false,
                material: material.gpu_word(),
            },
            &mut descriptor,
        )
        .unwrap();
    let fragment = descriptor.fragment.unwrap();
    let srgb = fragment.targets[0].as_ref().unwrap().format.is_srgb();
    let gamma = fragment.shader_defs.iter().any(
        |define| matches!(define, ShaderDefVal::Bool(name, true) if name == "ACTOR_GAMMA_BLEND"),
    );
    let plane = actor_raster::cube([16, 0, 16], true, false);
    let clear = actor_raster::raster_material_target(
        &gpu,
        &plane,
        crate::ActorMaterial {
            kind: EntityRenderMaterial::Dragon,
            ..Default::default()
        },
        false,
        [[0; 4]; 2],
        srgb,
        gamma,
    );
    let plate_alpha = 153;
    let plate = actor_raster::raster_material_target(
        &gpu,
        &plane,
        material,
        false,
        [[0, 0, 0, plate_alpha]; 2],
        srgb,
        gamma,
    );
    let pixel = (gpu_snapshot::SNAPSHOT_SIDE as usize / 2 * gpu_snapshot::SNAPSHOT_SIDE as usize
        + gpu_snapshot::SNAPSHOT_SIDE as usize / 2)
        * 4;
    for channel in 0..3 {
        let background = u32::from(clear[pixel + channel]);
        let expected = (background * u32::from(u8::MAX - plate_alpha) + u32::from(u8::MAX) / 2)
            / u32::from(u8::MAX);
        assert!(
            (i32::from(plate[pixel + channel]) - expected as i32).abs() <= 1,
            "black plate channel {channel}: background={background}, expected={expected}, actual={}",
            plate[pixel + channel],
        );
    }
}
