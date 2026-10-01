use assets::{
    CompiledEntityAssets, EntityAssetKind, EntityAssetSource, EntityAssetSymbol,
    RuntimeActorCatalog, encode_actor_catalog, encode_entity_blob,
};
use sha2::{Digest, Sha256};

/// Builds a single-cube geometry for UV eligibility checks.
fn geometry(size: [f32; 3], uv: assets::EntityGeometryUv) -> assets::EntityGeometry {
    use assets::{EntityGeometryBone, EntityGeometryCube, EntityGeometryScalar as Scalar};
    assets::EntityGeometry {
        identifier: "geometry.projectile".into(),
        inherits: None,
        source_index: 0,
        texture_width: 8,
        texture_height: 8,
        bones: vec![EntityGeometryBone {
            name: "body".into(),
            parent: None,
            pivot: None,
            rotation: None,
            mirror: None,
            inflate: None,
            never_render: None,
            reset: None,
            cubes: vec![EntityGeometryCube {
                origin: [Scalar::ZERO; 3],
                size: size.map(|v| Scalar::new(v).unwrap()),
                pivot: [Scalar::ZERO; 3],
                rotation: [Scalar::ZERO; 3],
                uv,
                inflate: Scalar::ZERO,
                mirror: false,
            }]
            .into(),
        }]
        .into(),
    }
}

#[test]
fn item_sprite_plane_does_not_require_the_unused_box_uv_envelope() {
    let geometry = geometry(
        [8.0, 8.0, 0.0],
        assets::EntityGeometryUv::Box([assets::EntityGeometryScalar::ZERO; 2]),
    );
    assert!(assets::neutral_actor_geometry_uvs_are_supported(
        &[geometry],
        0
    ));
}

#[test]
fn omitted_face_uv_size_uses_cube_dimensions_for_bounds() {
    use assets::{
        EntityGeometryFaceUv, EntityGeometryFaceUvs, EntityGeometryScalar as Scalar,
        EntityGeometryUv,
    };
    let geometry = geometry(
        [0.0, 5.0, 16.0],
        EntityGeometryUv::Faces(EntityGeometryFaceUvs {
            north: None,
            south: None,
            east: Some(EntityGeometryFaceUv {
                uv: [Scalar::ZERO; 2],
                uv_size: None,
            }),
            west: None,
            up: None,
            down: None,
        }),
    );
    assert!(!assets::neutral_actor_geometry_uvs_are_supported(
        &[geometry],
        0
    ));
}

fn entities() -> Box<[u8]> {
    encode_entity_blob(&CompiledEntityAssets {
        source_manifest_sha256: [1; 32],
        block_visual_count: 0,
        sources: vec![EntityAssetSource {
            path: "entity/example.entity.json".into(),
            source_bytes: 2,
            source_sha256: Sha256::digest(b"{}").into(),
        }]
        .into(),
        symbols: vec![EntityAssetSymbol {
            kind: EntityAssetKind::Entity,
            identifier: "minecraft:example".into(),
            source_index: 0,
            dependencies: Box::new([]),
        }]
        .into(),
        geometries: Box::new([]),
        animation_clips: Box::new([]),
        animation_channels: Box::new([]),
        animation_keyframes: Box::new([]),
        molang_symbols: Box::new([]),
        molang_expressions: Box::new([]),
        molang_ops: Box::new([]),
        molang_collections: Box::new([]),
        molang_collection_items: Box::new([]),
        controllers: Box::new([]),
        controller_states: Box::new([]),
        controller_animations: Box::new([]),
        controller_transitions: Box::new([]),
        rig_bindings: Box::new([]),
        rig_geometries: Box::new([]),
        rig_animations: Box::new([]),
        rig_controllers: Box::new([]),
        item_visuals: Box::new([]),
        item_visual_aliases: Box::new([]),
        render: Default::default(),
    })
    .unwrap()
}

#[test]
fn empty_generic_catalog_is_explicit_and_bound_to_exact_parent_carrier() {
    let entities = entities();
    let bytes = encode_actor_catalog(&entities, &[], &[]).unwrap();
    let runtime = RuntimeActorCatalog::decode(&bytes, &entities).unwrap();
    assert!(runtime.bindings().is_empty());
    assert!(runtime.textures().is_empty());
    assert_eq!(
        runtime.entity_identity(),
        <[u8; 32]>::from(Sha256::digest(&entities))
    );
    assert_eq!(bytes, encode_actor_catalog(&entities, &[], &[]).unwrap());
}

#[test]
fn malformed_rehashed_headers_cannot_bypass_carrier_policy_or_limits() {
    let entities = entities();
    let bytes = encode_actor_catalog(&entities, &[], &[]).unwrap();
    let mut old = bytes.clone();
    old[..8].copy_from_slice(b"MCBEACT1");
    old[8..12].copy_from_slice(&1u32.to_le_bytes());
    let end = old.len() - 32;
    let hash = Sha256::digest(&old[..end]);
    old[end..].copy_from_slice(&hash);
    assert!(RuntimeActorCatalog::decode(&old, &entities).is_err());
    for offset in [0, 8, 12, 16, 20, 24, 56, 88, 120] {
        let mut modified = bytes.clone();
        modified[offset] ^= 1;
        let end = modified.len() - 32;
        let digest = Sha256::digest(&modified[..end]);
        modified[end..].copy_from_slice(&digest);
        assert!(
            RuntimeActorCatalog::decode(&modified, &entities).is_err(),
            "offset {offset}"
        );
    }
    for length in [0, 8, 128, bytes.len() - 1] {
        assert!(RuntimeActorCatalog::decode(&bytes[..length], &entities).is_err());
    }
}
