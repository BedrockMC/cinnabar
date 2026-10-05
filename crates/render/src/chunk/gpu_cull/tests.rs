use super::model::{
    ARGS_WORDS, CullCamera, CullPhase, CullRecord, CullStream, args_region, args_words,
    reference_args, slot_enabled,
};
use super::*;

fn allocation(layout: CubeQuadLayout, metadata_index: u32) -> GpuChunkAllocation {
    GpuChunkAllocation {
        key: SubChunkKey::new(0, 1, 4, -1),
        generation: 1,
        tint_identity: ChunkBiomeTintIdentity::default(),
        quad_range: 100..112,
        cube_layout: layout,
        cube_lighting_range: Some(200..224),
        model_range: Some(224..232),
        model_lighting_range: Some(232..240),
        model_draw_range: Some(240..250),
        transparent_model_draw_range: None,
        liquid_range: Some(250..270),
        liquid_lighting_range: Some(270..280),
        has_depth_liquid: true,
        has_transparent_liquid: false,
        depth_liquid_range: Some(5..9),
        metadata_index,
    }
}

fn cpu_args(allocation: &GpuChunkAllocation, camera: [f64; 3]) -> [Vec<[u32; 5]>; 4] {
    let words = |draw: DrawIndexedIndirectArgs| {
        [
            draw.index_count,
            draw.instance_count,
            draw.first_index,
            draw.base_vertex as u32,
            draw.first_instance,
        ]
    };
    [
        solid_indirect_commands(allocation, Some(camera))
            .into_iter()
            .flatten()
            .map(words)
            .collect(),
        cutout_indirect_command(allocation)
            .map(words)
            .into_iter()
            .collect(),
        model_mdi_draw_command(allocation)
            .map(words)
            .into_iter()
            .collect(),
        depth_liquid_mdi_draw_command(allocation)
            .map(words)
            .into_iter()
            .collect(),
    ]
}

/// A record drawn through the cull kernels issues exactly the CPU path's indirect commands.
#[test]
fn records_reproduce_every_cpu_indirect_command_from_every_side() {
    let index_counts = [
        STATIC_QUAD_INDICES.len() as u32,
        STATIC_QUAD_INDICES.len() as u32,
        MODEL_INDEX_COUNT,
        STATIC_QUAD_INDICES.len() as u32,
    ];
    let layouts = [
        CubeQuadLayout::from_solid_counts([1, 1, 2, 2, 1, 1]),
        CubeQuadLayout::from_solid_counts([2, 0, 0, 3, 1, 0]),
        CubeQuadLayout::from_solid_counts([2; 6]),
        CubeQuadLayout::default(),
        CubeQuadLayout::from_solid_counts([3; 6]),
    ];
    let origin = chunk_origin(SubChunkKey::new(0, 1, 4, -1)).map(f64::from);
    let offsets = [-7.5, 0.0, 0.25, 8.0, 15.0, 16.0, 16.5, 40.0];
    for layout in layouts {
        let allocation = allocation(layout, 3);
        let record = cull_record(&allocation, None);
        for x in offsets {
            for y in offsets {
                for z in offsets {
                    let camera = [origin[0] + x, origin[1] + y, origin[2] + z];
                    let gpu = reference_args(
                        &[
                            CullRecord::default(),
                            CullRecord::default(),
                            CullRecord::default(),
                            record,
                        ],
                        CullCamera::new(Some(camera)),
                        index_counts,
                        |_| true,
                    );
                    assert_eq!(
                        gpu,
                        cpu_args(&allocation, camera),
                        "{layout:?} at {camera:?}"
                    );
                }
            }
        }
    }
    let mut invalid = allocation(CubeQuadLayout::default(), 3);
    invalid.cube_lighting_range = None;
    invalid.model_range = None;
    invalid.has_depth_liquid = false;
    assert!(!cull_record(&invalid, None).is_live());
}

#[test]
fn split_eye_selects_the_same_faces_as_the_f64_eye() {
    let origin = [-32, 64, 48];
    let values = [
        -40.0, -32.0, -31.999, -17.0, -16.5, -16.0, 0.0, 47.0, 48.0, 48.25, 64.0, 64.5, 80.0,
    ];
    for x in values {
        for y in values.map(|value| value + 64.0) {
            for z in values.map(|value| value + 32.0) {
                let eye = [x, y, z];
                let expected = (0..6)
                    .filter(|&face| {
                        meshing::sub_chunk_facing_faces(origin, eye).contains(Face::ALL[face])
                    })
                    .fold(0_u8, |mask, face| mask | 1 << (Face::ALL[face] as u8));
                assert_eq!(
                    CullCamera::new(Some(eye)).facing(origin),
                    expected,
                    "{eye:?}"
                );
            }
        }
    }
    assert_eq!(CullCamera::new(None).facing(origin), 0x3f);
    assert_eq!(
        CullCamera::new(Some([f64::NAN, 0.0, 0.0])).facing(origin),
        0x3f
    );
}

#[test]
fn quad_bounds_contain_the_rasterised_quad() {
    for face in Face::ALL {
        let quad = PackedQuad::new([3, 14, 0], face, 4, 2, 0);
        let [low, high] = quad_bounds(&quad);
        assert_eq!(low, [3, 14, 0]);
        // The +Y plane sits one block above the origin; extents clamp to the sub-chunk.
        assert!(high[1] >= 15 && high.iter().all(|&value| value <= SIDE));
        assert!(high[0] >= 7 && high[2] >= 4);
    }
}

#[test]
fn args_regions_are_disjoint_and_fit_the_buffer() {
    let capacity = 1024;
    let mut regions = CullPhase::ALL
        .into_iter()
        .flat_map(|phase| {
            CullStream::ALL.map(|stream| {
                let start = args_region(capacity, phase, stream);
                (
                    start,
                    start + stream.draws_per_record() * capacity * ARGS_WORDS,
                )
            })
        })
        .collect::<Vec<_>>();
    regions.sort_unstable();
    assert!(regions.windows(2).all(|pair| pair[0].1 == pair[1].0));
    assert_eq!(regions[0].0, 0);
    assert_eq!(u64::from(regions.last().unwrap().1), args_words(capacity));
}

/// The GPU path is selected only where multi-draw-indirect-count is native.
#[test]
fn only_count_capable_indirect_devices_cull_on_the_gpu() {
    let count = WgpuFeatures::MULTI_DRAW_INDIRECT_COUNT;
    let compute = DownlevelFlags::COMPUTE_SHADERS;
    let mdi = ChunkDrawMode::MultiDrawIndirect;
    assert!(gpu_cull_supported(mdi, count, compute, false));
    assert!(!gpu_cull_supported(mdi, count, compute, true));
    assert!(!gpu_cull_supported(
        mdi,
        WgpuFeatures::empty(),
        compute,
        false
    ));
    assert!(!gpu_cull_supported(
        ChunkDrawMode::Direct,
        count,
        compute,
        false
    ));
    assert!(!gpu_cull_supported(
        mdi,
        count,
        DownlevelFlags::empty(),
        false
    ));
}

#[test]
fn slot_table_tracks_moves_removals_cave_visibility_and_tint() {
    let entity = |index: u32| Entity::from_raw_u32(index + 1).unwrap();
    let live = cull_record(&allocation(CubeQuadLayout::default(), 0), None);
    let tint = ChunkBiomeTintIdentity::default();
    let mut hidden = HashSet::new();
    let mut table = CullSlots::default();
    table.set_tint(tint, &hidden);
    table.update(entity(0), 2, tint, live, &hidden);
    table.update(entity(1), 0, tint, live, &hidden);
    assert_eq!(table.slot_count(), 3);
    assert!(slot_enabled(table.enabled(), 2) && slot_enabled(table.enabled(), 0));
    assert_eq!(table.take_dirty(), [0, 1, 2]);

    // A replacement allocation moves the entity; the old slot is cleared and re-uploaded.
    table.update(entity(0), 1, tint, live, &hidden);
    assert!(!slot_enabled(table.enabled(), 2) && !table.records()[2].is_live());
    table.trim();
    assert_eq!(table.slot_count(), 2);
    assert_eq!(table.take_dirty(), [1]);

    hidden.insert(entity(1));
    table.refresh_entity(entity(1), &hidden);
    assert!(!slot_enabled(table.enabled(), 0));
    hidden.clear();
    table.refresh_entity(entity(1), &hidden);
    assert!(slot_enabled(table.enabled(), 0));

    let stale = ChunkBiomeTints::with_revision(Arc::from([]), 9).table_identity();
    table.set_tint(stale, &hidden);
    assert!(!slot_enabled(table.enabled(), 0) && !slot_enabled(table.enabled(), 1));

    // Regrowing past a trimmed slot re-uploads it, so no stale GPU record survives.
    table.set_tint(tint, &hidden);
    table.remove(entity(0));
    table.trim();
    table.take_dirty();
    table.update(entity(2), 3, tint, live, &hidden);
    assert_eq!(table.take_dirty(), [1, 2, 3]);
}
