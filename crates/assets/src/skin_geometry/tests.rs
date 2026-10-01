use super::*;

const PATCH: &str = r#"{"geometry":{"default":"geometry.npc"}}"#;

#[test]
fn modern_and_inherited_legacy_models_keep_authored_visibility_bounds() {
    let modern = r#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.npc","visible_bounds_width":3,"visible_bounds_height":4,"visible_bounds_offset":[0,2,0]},"bones":[{"name":"body"}]}]}"#;
    let legacy = r#"{"format_version":"1.8.0","geometry.base":{"visible_bounds_width":3,"visible_bounds_height":4,"visible_bounds_offset":[0,2,0],"bones":[{"name":"body"}]},"geometry.npc:geometry.base":{"bones":[{"name":"head"}]}}"#;
    for source in [modern, legacy] {
        let model = parse_skin_geometry(PATCH, source).unwrap().unwrap();
        assert_eq!(
            model.visible_bounds,
            Some(SkinGeometryBounds {
                center: [0.0, 2.0, 0.0],
                half_extents: [1.5, 2.0, 1.5]
            })
        );
    }
}

#[test]
fn animated_geometry_alias_selects_its_model_and_has_a_distinct_mesh_digest() {
    let patch = r#"{"geometry":{"default":"geometry.body","animated_face":"geometry.face"}}"#;
    let data = r#"{"format_version":"1.14.0","minecraft:geometry":[
        {"description":{"identifier":"geometry.body","texture_width":256,"texture_height":256},"bones":[{"name":"body"}]},
        {"description":{"identifier":"geometry.face","texture_width":32,"texture_height":64},"bones":[{"name":"head"}]}
    ]}"#;
    let body = parse_skin_geometry(patch, data).unwrap().unwrap();
    let face = parse_skin_geometry_layer(patch, data, "animated_face")
        .unwrap()
        .unwrap();
    assert_eq!(face.bones[0].name.as_ref(), "head");
    assert_eq!((face.texture_width, face.texture_height), (32, 64));
    assert_ne!(
        body.digest, face.digest,
        "different layer models cannot share a cached mesh"
    );
}

// Remote skin JSON carries editor fields and odd cubes; they are ignored or dropped, not fatal.
#[test]
fn modern_geometry_ignores_unknown_fields_and_drops_malformed_cubes() {
    let data = r#"{"format_version":"1.12.0","debug":true,"minecraft:geometry":[{
        "description":{"identifier":"geometry.npc","texture_width":128,"texture_height":128,"visible_bounds_width":2},
        "bones":[
          {"name":"body","pivot":[0,24,0],"META_BoneType":"base","cubes":[
            {"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]},
            {"origin":[0,0,0],"size":[0,0,4],"uv":[0,0]},
            {"origin":[-1,30,-1],"size":[2,2,2],"uv":{"north":{"uv":[1,2],"uv_size":[2,2],"material_instance":"x"}}}]},
          {"name":"horn","parent":"missing","poly_mesh":{},"cubes":[{"origin":[0,32,0],"size":[1,4,1]}]}]}]}"#;
    let geometry = parse_skin_geometry(PATCH, data).unwrap().unwrap();
    assert_eq!(
        (geometry.texture_width, geometry.texture_height),
        (128, 128)
    );
    assert_eq!(geometry.bones.len(), 2);
    assert_eq!(
        geometry.bones[0].cubes.len(),
        2,
        "the degenerate cube is dropped"
    );
    assert!(matches!(
        geometry.bones[0].cubes[1].uv,
        EntityGeometryUv::Faces(_)
    ));
    assert_eq!(
        geometry.bones[1].parent, None,
        "an unknown parent becomes a root"
    );
}

// Legacy keys inherit within the skin's own geometries; modern ones may not inherit at all.
#[test]
fn legacy_inheritance_overlays_within_the_skin_json() {
    let data = r#"{"format_version":"1.8.0",
        "geometry.base":{"texturewidth":64,"textureheight":64,"bones":[
            {"name":"body","pivot":[0,24,0],"cubes":[{"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]}]},
            {"name":"head","parent":"body","cubes":[{"origin":[-4,24,-4],"size":[8,8,8],"uv":[0,0]}]},
            {"name":"hat","parent":"head","cubes":[{"origin":[-4,24,-4],"size":[8,8,8],"uv":[32,0]}]}]},
        "geometry.NPC:geometry.base":{"bones":[
            {"name":"head","cubes":[{"origin":[-5,24,-5],"size":[10,10,10],"uv":[0,0]}]},
            {"name":"hat","reset":true}]}}"#;
    let geometry = parse_skin_geometry(PATCH, data).unwrap().unwrap();
    assert_eq!(geometry.bones.len(), 3);
    assert_eq!(geometry.bones[1].cubes[0].size[0].get(), 10.0);
    assert!(
        geometry.bones[2].cubes.is_empty(),
        "reset hides the inherited hat"
    );
    let modern = r#"{"format_version":"1.12.0","minecraft:geometry":[
        {"description":{"identifier":"geometry.npc:geometry.base"},"bones":[{"name":"body"}]}]}"#;
    assert_eq!(
        parse_skin_geometry(PATCH, modern),
        Err(SkinGeometryError::MissingGeometry)
    );
}

#[test]
fn default_geometry_and_absent_data_need_no_custom_model() {
    assert_eq!(parse_skin_geometry(PATCH, ""), Ok(None));
    assert_eq!(parse_skin_geometry(PATCH, "null"), Ok(None));
    assert_eq!(parse_skin_geometry("", "{}"), Ok(None));
    assert_eq!(
        parse_skin_geometry(
            PATCH,
            r#"{"format_version":"1.12.0","minecraft:geometry":[]}"#
        ),
        Err(SkinGeometryError::MissingGeometry)
    );
}

#[test]
fn unusable_models_are_rejected() {
    let cyclic = r#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.npc"},
        "bones":[{"name":"a","parent":"b"},{"name":"b","parent":"a"}]}]}"#;
    assert_eq!(
        parse_skin_geometry(PATCH, cyclic),
        Err(SkinGeometryError::CyclicHierarchy)
    );
    let bones = (0..=MAX_SKIN_GEOMETRY_BONES)
        .map(|index| format!(r#"{{"name":"b{index}"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let crowded = format!(
        r#"{{"format_version":"1.12.0","minecraft:geometry":[{{"description":{{"identifier":"geometry.npc"}},"bones":[{bones}]}}]}}"#
    );
    assert_eq!(
        parse_skin_geometry(PATCH, &crowded),
        Err(SkinGeometryError::TooManyBones)
    );
    assert_eq!(
        parse_skin_geometry(PATCH, "not json"),
        Err(SkinGeometryError::Json)
    );
}
