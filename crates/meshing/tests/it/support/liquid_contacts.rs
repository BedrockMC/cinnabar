use std::collections::HashMap;

use super::{
    AIR, CROSS, GLASS, NON_WATER_LIQUID, OTHER_LIQUID, SOLID, WATER_SOURCE, blocks, mesh,
    packed_storage, runtime_assets, sub_chunk,
};
use assets::{BlockFace, MATERIAL_FLAG_WATER_TINT, NetworkIdMode};
use meshing::{BlockClassifier, ChunkMesh, ContributorResolver, Face};
use world::{MeshNeighbourhood, MeshSample, SubChunk};

const ORIGIN: [u8; 3] = [8, 8, 8];
const CONTACTS: [(Face, [i32; 3]); 5] = [
    (Face::NegativeX, [-1, 0, 0]),
    (Face::PositiveX, [1, 0, 0]),
    (Face::NegativeZ, [0, 0, -1]),
    (Face::PositiveZ, [0, 0, 1]),
    (Face::NegativeY, [0, -1, 0]),
];

fn adjacent(origin: [u8; 3], offset: [i32; 3]) -> [i32; 3] {
    std::array::from_fn(|axis| i32::from(origin[axis]) + offset[axis])
}

fn has_face(mesh: &ChunkMesh, face: Face) -> bool {
    mesh.liquid_quads()
        .iter()
        .any(|quad| quad.origin() == ORIGIN && quad.face() == face)
}

#[test]
fn classic_water_hides_full_cube_and_liquid_contacts() {
    for neighbour in [GLASS, OTHER_LIQUID, NON_WATER_LIQUID, SOLID] {
        for (face, offset) in CONTACTS {
            let position = adjacent(ORIGIN, offset).map(|coordinate| coordinate as u8);
            let output = mesh(&blocks(&[(WATER_SOURCE, ORIGIN), (neighbour, position)]));
            assert!(!has_face(&output, face), "contact {neighbour} {face:?}");
            assert!(has_face(&output, Face::PositiveY));
            assert_eq!(output.liquid_lighting().len(), output.liquid_quads().len());
            assert!(
                output
                    .liquid_quads()
                    .iter()
                    .enumerate()
                    .all(|(index, quad)| quad.lighting_index() == index as u32)
            );
        }
    }
}

#[test]
fn classic_water_keeps_faces_beside_thin_non_occluding_primary_geometry() {
    for (face, offset) in CONTACTS {
        let position = adjacent(ORIGIN, offset).map(|coordinate| coordinate as u8);
        let output = mesh(&blocks(&[(WATER_SOURCE, ORIGIN), (CROSS, position)]));
        let quad = output
            .liquid_quads()
            .iter()
            .find(|quad| quad.origin() == ORIGIN && quad.face() == face)
            .expect("water remains visible beside thin geometry");
        assert!(!quad.is_two_sided());
    }
}

#[test]
fn non_water_liquid_keeps_single_winding_transparent_primary_contacts() {
    for neighbour in [GLASS, CROSS, OTHER_LIQUID, WATER_SOURCE] {
        let output = mesh(&blocks(&[
            (NON_WATER_LIQUID, ORIGIN),
            (neighbour, [9, 8, 8]),
            (neighbour, [8, 7, 8]),
        ]));
        for face in [Face::PositiveX, Face::NegativeY] {
            let quad = output
                .liquid_quads()
                .iter()
                .find(|quad| quad.origin() == ORIGIN && quad.face() == face)
                .expect("native non-water contact face");
            assert!(quad.is_depth_writing());
            assert!(!quad.is_two_sided());
        }
    }
}

#[test]
fn primary_water_and_additional_water_do_not_duplicate_exposed_surfaces() {
    let position = (ORIGIN, 1);
    let output = mesh(&sub_chunk(vec![
        packed_storage(1, &[AIR, WATER_SOURCE], &[position]),
        packed_storage(1, &[AIR, WATER_SOURCE], &[position]),
    ]));
    assert_eq!(output.liquid_quads().len(), Face::ALL.len());
    for face in Face::ALL {
        assert!(has_face(&output, face));
    }
}

#[test]
fn neighbour_primary_controls_contact_admission_not_its_additional_water() {
    for primary in [AIR, GLASS, CROSS, SOLID, OTHER_LIQUID] {
        for extra in [WATER_SOURCE, OTHER_LIQUID] {
            for (face, offset) in CONTACTS {
                let position = adjacent(ORIGIN, offset).map(|coordinate| coordinate as u8);
                let output = mesh(&sub_chunk(vec![
                    packed_storage(1, &[AIR, primary], &[(position, 1)]),
                    packed_storage(
                        2,
                        &[AIR, WATER_SOURCE, extra],
                        &[(ORIGIN, 1), (position, 2)],
                    ),
                ]));
                assert_eq!(
                    has_face(&output, face),
                    matches!(primary, AIR | CROSS) && extra != WATER_SOURCE,
                    "primary {primary} extra {extra} face {face:?}"
                );
                if has_face(&output, face) {
                    let quad = output
                        .liquid_quads()
                        .iter()
                        .find(|quad| quad.origin() == ORIGIN && quad.face() == face)
                        .unwrap();
                    assert_eq!(
                        quad.is_two_sided(),
                        primary == AIR && face != Face::NegativeY
                    );
                }
                assert!(has_face(&output, Face::PositiveY));
            }
        }
    }
}

pub(super) fn assert_thin_primary_contacts(chunks: &[SubChunk], output: &ChunkMesh) {
    let mut neighbourhood = MeshNeighbourhood::new(&chunks[13]);
    let mut index = 0;
    for x in -1..=1_i8 {
        for y in -1..=1_i8 {
            for z in -1..=1_i8 {
                if [x, y, z] != [0, 0, 0] {
                    assert!(neighbourhood.insert([x, y, z], &chunks[index]));
                }
                index += 1;
            }
        }
    }
    let classifier = BlockClassifier::new(AIR);
    let assets = runtime_assets();
    let resolver =
        ContributorResolver::new(classifier, assets, NetworkIdMode::Sequential, &chunks[13]);
    let quads = output
        .liquid_quads()
        .iter()
        .map(|quad| ((quad.origin(), quad.face()), quad))
        .collect::<HashMap<_, _>>();
    let materials = |id| {
        BlockFace::ALL.map(|face| {
            assets
                .resolve(NetworkIdMode::Sequential, id)
                .face(face)
                .material_id()
        })
    };
    let mut admitted = 0;
    for x in 0..16_u8 {
        for y in 0..16_u8 {
            for z in 0..16_u8 {
                let origin = [x, y, z];
                let Some(liquid) = resolver.resolve(origin).liquid_network_value() else {
                    continue;
                };
                let identity = materials(liquid);
                if assets.material(identity[BlockFace::Up as usize]).flags
                    & MATERIAL_FLAG_WATER_TINT
                    == 0
                {
                    continue;
                }
                for (face, offset) in CONTACTS {
                    let position = adjacent(origin, offset);
                    if neighbourhood.liquid_sample(0, position) != MeshSample::Block(CROSS) {
                        continue;
                    }
                    let (chunk, local) = neighbourhood.liquid_block_source(position).unwrap();
                    let neighbour = ContributorResolver::resolve_direct(
                        classifier,
                        assets,
                        NetworkIdMode::Sequential,
                        chunk,
                        local,
                    );
                    let matching_water = neighbour
                        .liquid_network_value()
                        .is_some_and(|id| materials(id) == identity);
                    let quad = quads.get(&(origin, face));
                    assert_eq!(
                        quad.is_some(),
                        !matching_water,
                        "thin primary at {position:?}, water at {origin:?}"
                    );
                    if let Some(quad) = quad {
                        assert!(!quad.is_two_sided());
                        admitted += 1;
                    }
                }
            }
        }
    }
    assert!(
        admitted > 0,
        "dense fixture covers visible thin-primary contacts"
    );
    assert_eq!(output.liquid_lighting().len(), output.liquid_quads().len());
    assert!(
        output
            .liquid_quads()
            .iter()
            .enumerate()
            .all(|(index, quad)| quad.lighting_index() == index as u32)
    );
}
