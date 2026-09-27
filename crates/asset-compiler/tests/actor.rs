use std::{fs, path::Path};

use asset_compiler::{compile_actor_assets, compile_entity_assets};
use assets::{RuntimeActorCatalog, encode_actor_catalog, encode_entity_blob};
use image::{Rgba, RgbaImage};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const MANIFEST: &[u8] = include_bytes!("../../../assets/vanilla-source.json");

fn write(root: &Path, path: &str, bytes: &[u8]) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn pack(alpha: u8, material: &str, conditional: bool) -> TempDir {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    write(root, "entity/example.entity.json", format!(r#"{{"format_version":"1.8.0","minecraft:client_entity":{{"description":{{"identifier":"minecraft:example","geometry":{{"default":"geometry.example"}},"materials":{{"default":"{material}"}},"textures":{{"default":"textures/entity/example"}},"render_controllers":["controller.render.example"]}}}}}}"#).as_bytes());
    write(root, "models/entity/example.geo.json", br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.example","texture_width":16,"texture_height":16},"bones":[{"name":"root","cubes":[{"origin":[0,0,0],"size":[0,2,7],"uv":[0,0]}]}]}]}"#);
    write(
        root,
        "animations/empty.json",
        br#"{"format_version":"1.8.0","animations":{}}"#,
    );
    write(
        root,
        "animation_controllers/empty.json",
        br#"{"format_version":"1.10.0","animation_controllers":{}}"#,
    );
    let geometry = if conditional {
        "query.is_alive ? Geometry.default : Geometry.default"
    } else {
        "Geometry.default"
    };
    write(root, "render_controllers/example.json", format!(r#"{{"format_version":"1.8.0","render_controllers":{{"controller.render.example":{{"geometry":"{geometry}","materials":[{{"*":"Material.default"}}],"textures":["Texture.default"]}}}}}}"#).as_bytes());
    let mut image = RgbaImage::from_pixel(16, 16, Rgba([17, 31, 47, 255]));
    image.put_pixel(0, 0, Rgba([0, 0, 0, alpha]));
    fs::create_dir_all(root.join("textures/entity")).unwrap();
    image
        .save(root.join("textures/entity/example.png"))
        .unwrap();
    temporary
}

#[test]
fn generic_actor_carrier_resolves_unconditional_route_and_exact_entity_identity() {
    let pack = pack(0, "entity_alphatest", false);
    let first = compile_actor_assets(pack.path(), MANIFEST).unwrap();
    let second = compile_actor_assets(pack.path(), MANIFEST).unwrap();
    assert_eq!(first.bytes, second.bytes);
    let entities =
        encode_entity_blob(&compile_entity_assets(pack.path(), MANIFEST).unwrap()).unwrap();
    let runtime = RuntimeActorCatalog::decode(&first.bytes, &entities).unwrap();
    assert_eq!(runtime.bindings().len(), 1);
    assert_eq!(runtime.textures().len(), 1);
    assert_eq!(
        (runtime.textures()[0].width, runtime.textures()[0].height),
        (16, 16)
    );
    assert_eq!(runtime.textures()[0].rgba8[3], 0);
    let mut stale = entities.to_vec();
    stale[24] ^= 1;
    assert!(RuntimeActorCatalog::decode(&first.bytes, &stale).is_err());
}

#[test]
fn authored_selection_and_scripts_cannot_bypass_neutral_admission() {
    for mutation in [
        serde_json::json!({"render_controllers":[{"controller.render.example":"0"}]}),
        serde_json::json!({"render_controllers":["controller.render.example","controller.render.example"]}),
        serde_json::json!({"scripts":{"scale":"0"}}),
        serde_json::json!({"scripts":{"initialize":["variable.x=1;"]}}),
        serde_json::json!({"scripts":{"pre_animation":["variable.x=1;"]}}),
        serde_json::json!({"scripts":{"animate":[{"missing":"1"}]}}),
    ] {
        let pack = pack(0, "entity_alphatest", false);
        let path = pack.path().join("entity/example.entity.json");
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (key, entry) in mutation.as_object().unwrap() {
            value["minecraft:client_entity"]["description"][key] = entry.clone();
        }
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
        assert_eq!(compiled.report.bindings, 0, "{mutation}");
        assert!(!compiled.report.fallbacks.is_empty());
    }
}

fn animated_pack(weight: &str) -> TempDir {
    let pack = pack(0, "entity_alphatest", false);
    let path = pack.path().join("entity/example.entity.json");
    let mut entity: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    entity["minecraft:client_entity"]["description"]["animations"] =
        serde_json::json!({"swim":"animation.example.swim"});
    entity["minecraft:client_entity"]["description"]["animation_controllers"] =
        serde_json::json!([{"general":"controller.animation.example"}]);
    fs::write(path, serde_json::to_vec(&entity).unwrap()).unwrap();
    write(pack.path(), "animations/empty.json", br#"{"format_version":"1.8.0","animations":{"animation.example.swim":{"loop":true,"animation_length":2,"bones":{"root":{"rotation":{"0":[0,-20,0],"1":[0,20,0],"2":[0,-20,0]}}}}}}"#);
    write(pack.path(), "animation_controllers/empty.json", serde_json::to_vec(&serde_json::json!({"format_version":"1.10.0","animation_controllers":{"controller.animation.example":{"initial_state":"default","states":{"default":{"animations":[{"swim":weight}]}}}}})).unwrap().as_slice());
    pack
}

#[test]
fn legacy_controller_only_query_pose_is_explicit_rest_and_cannot_be_forged_literal() {
    for (weight, mode) in [
        ("1", assets::ActorPoseMode::CompiledLiteral),
        (
            "math.min(1.0, query.modified_move_speed * 10)",
            assets::ActorPoseMode::RestPose,
        ),
    ] {
        let pack = animated_pack(weight);
        let entities = compile_entity_assets(pack.path(), MANIFEST).unwrap();
        assert_eq!(entities.rig_geometries[0].animation_count, 0);
        assert_eq!(entities.rig_geometries[0].controller_count, 1);
        let entity_bytes = encode_entity_blob(&entities).unwrap();
        let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
        let catalog = RuntimeActorCatalog::decode(&compiled.bytes, &entity_bytes).unwrap();
        assert_eq!(catalog.bindings().len(), 1);
        assert_eq!(catalog.bindings()[0].pose_mode, mode);
        assert_eq!(
            compiled.report.rest_pose_bindings,
            usize::from(mode == assets::ActorPoseMode::RestPose)
        );
        if mode == assets::ActorPoseMode::RestPose {
            assert!(
                compiled
                    .report
                    .fallbacks
                    .iter()
                    .any(|v| v.reason.as_ref() == "pose_expression_unverified")
            );
            let mut forged = catalog.bindings().to_vec();
            forged[0].pose_mode = assets::ActorPoseMode::CompiledLiteral;
            assert!(encode_actor_catalog(&entity_bytes, catalog.textures(), &forged).is_err());
            let mut bytes = compiled.bytes.clone();
            let mode_offset = 128 + 40 + 16 * 16 * 4 + 24;
            for invalid in [0u32, 9] {
                bytes[mode_offset..mode_offset + 4].copy_from_slice(&invalid.to_le_bytes());
                let end = bytes.len() - 32;
                let hash = Sha256::digest(&bytes[..end]);
                bytes[end..].copy_from_slice(&hash);
                assert!(RuntimeActorCatalog::decode(&bytes, &entity_bytes).is_err());
            }
        }
    }
}

#[test]
fn ignored_geometry_clip_and_controller_semantics_are_observable_rejections() {
    for (file, pointer, value) in [
        (
            "models/entity/example.geo.json",
            "/minecraft:geometry/0/bones/0/binding",
            serde_json::json!("query.is_alive"),
        ),
        (
            "models/entity/example.geo.json",
            "/minecraft:geometry/0/bones/0/bind_pose_rotation",
            serde_json::json!([0, 30, 0]),
        ),
        (
            "models/entity/example.geo.json",
            "/minecraft:geometry/0/bones/0/reset",
            serde_json::json!(true),
        ),
        (
            "models/entity/example.geo.json",
            "/minecraft:geometry/0/bones/0/texture_meshes",
            serde_json::json!([]),
        ),
        (
            "animations/empty.json",
            "/animations/animation.example.swim/start_delay",
            serde_json::json!(1),
        ),
        (
            "animations/empty.json",
            "/animations/animation.example.swim/anim_time_update",
            serde_json::json!("query.life_time"),
        ),
        (
            "animations/empty.json",
            "/animations/animation.example.swim/bones/root/relative_to",
            serde_json::json!({"rotation":"entity"}),
        ),
        (
            "animations/empty.json",
            "/animations/animation.example.swim/bones/root/scale",
            serde_json::json!([1, 2, 1]),
        ),
        (
            "animation_controllers/empty.json",
            "/animation_controllers/controller.animation.example/states/default/blend_transition",
            serde_json::json!(0.2),
        ),
        (
            "animation_controllers/empty.json",
            "/animation_controllers/controller.animation.example/states/default/variables",
            serde_json::json!({}),
        ),
    ] {
        let pack = animated_pack("1");
        let path = pack.path().join(file);
        let mut json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        json.pointer_mut(parent).unwrap()[key] = value;
        fs::write(path, serde_json::to_vec(&json).unwrap()).unwrap();
        let result = compile_actor_assets(pack.path(), MANIFEST).unwrap();
        assert_eq!(result.report.bindings, 0, "{pointer}");
        assert!(
            result
                .report
                .fallbacks
                .iter()
                .any(|v| v.reason.as_ref() == "unsupported_authored_state")
        );
    }
}

#[test]
fn handled_never_render_is_retained_instead_of_rejected_as_unknown_state() {
    let pack = pack(0, "entity_alphatest", false);
    let path = pack.path().join("models/entity/example.geo.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["minecraft:geometry"][0]["bones"][0]["neverRender"] = serde_json::json!(true);
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    let compiled = compile_entity_assets(pack.path(), MANIFEST).unwrap();
    assert_eq!(compiled.geometries[0].bones[0].never_render, Some(true));
    assert_eq!(
        compile_actor_assets(pack.path(), MANIFEST)
            .unwrap()
            .report
            .bindings,
        1
    );
}

#[test]
fn bone_defaults_are_rejected_even_with_cube_overrides_or_inherited_sources() {
    for (property, value) in [
        ("inflate", serde_json::json!(1)),
        ("mirror", serde_json::json!(true)),
    ] {
        for inherited in [false, true] {
            for explicit_cube_override in [false, true] {
                let pack = pack(0, "entity_alphatest", false);
                let path = pack.path().join("models/entity/example.geo.json");
                let mut bone = serde_json::json!({"name":"root","cubes":[{"origin":[0,0,0],"size":[1,2,7],"uv":[0,0]}]});
                bone[property] = value.clone();
                if explicit_cube_override {
                    bone["cubes"][0][property] = if property == "inflate" {
                        serde_json::json!(0)
                    } else {
                        serde_json::json!(false)
                    };
                }
                let definition =
                    serde_json::json!({"texturewidth":16,"textureheight":16,"bones":[bone]});
                let geometry = if inherited {
                    serde_json::json!({"format_version":"1.8.0","geometry.parent":definition,"geometry.example:geometry.parent":{"texturewidth":16,"textureheight":16,"bones":[]}})
                } else {
                    serde_json::json!({"format_version":"1.8.0","geometry.example":definition})
                };
                fs::write(path, serde_json::to_vec(&geometry).unwrap()).unwrap();
                let parent = compile_entity_assets(pack.path(), MANIFEST).unwrap();
                let bytes = encode_entity_blob(&parent).unwrap();
                let result = compile_actor_assets(pack.path(), MANIFEST).unwrap();
                let catalog = RuntimeActorCatalog::decode(&result.bytes, &bytes).unwrap();
                assert!(
                    catalog.bindings().is_empty(),
                    "{property} inherited={inherited} override={explicit_cube_override}"
                );
                assert!(
                    result
                        .report
                        .fallbacks
                        .iter()
                        .any(|v| v.reason.as_ref() == "unsupported_authored_state")
                );
            }
        }
    }
}

#[test]
fn ordinary_cube_mirror_and_default_bone_flags_remain_admissible() {
    let pack = pack(0, "entity_alphatest", false);
    let path = pack.path().join("models/entity/example.geo.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let bone = &mut json["minecraft:geometry"][0]["bones"][0];
    bone["inflate"] = serde_json::json!(0);
    bone["mirror"] = serde_json::json!(false);
    bone["cubes"][0]["mirror"] = serde_json::json!(true);
    fs::write(path, serde_json::to_vec(&json).unwrap()).unwrap();
    let entities =
        encode_entity_blob(&compile_entity_assets(pack.path(), MANIFEST).unwrap()).unwrap();
    let result = compile_actor_assets(pack.path(), MANIFEST).unwrap();
    assert_eq!(
        RuntimeActorCatalog::decode(&result.bytes, &entities)
            .unwrap()
            .bindings()
            .len(),
        1
    );
}

#[test]
fn unsupported_actor_pixels_material_and_selection_are_counted_not_quantized() {
    for (alpha, material, conditional, reason) in [
        (128, "entity_alphatest", false, "fractional_alpha"),
        (0, "unreviewed_material", false, "unsupported_material"),
        (0, "entity_alphatest", true, "conditional_selection"),
    ] {
        let pack = pack(alpha, material, conditional);
        let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
        assert_eq!(compiled.report.bindings, 0);
        assert!(
            compiled
                .report
                .fallbacks
                .iter()
                .any(|entry| entry.reason.as_ref() == reason)
        );
    }
}

#[test]
fn actor_pixels_are_not_cropped_to_geometry_dimensions() {
    let pack = pack(0, "entity_alphatest", false);
    RgbaImage::from_pixel(32, 16, Rgba([1, 2, 3, 255]))
        .save(pack.path().join("textures/entity/example.png"))
        .unwrap();
    let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
    assert_eq!(compiled.report.bindings, 0);
    assert!(
        compiled
            .report
            .fallbacks
            .iter()
            .any(|entry| entry.reason.as_ref() == "texture_dimensions")
    );
}

#[test]
fn runtime_rejects_rehashed_untrusted_pixels_and_binding_substitutions() {
    let pack = pack(0, "entity_alphatest", false);
    let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
    let entities =
        encode_entity_blob(&compile_entity_assets(pack.path(), MANIFEST).unwrap()).unwrap();
    let catalog = RuntimeActorCatalog::decode(&compiled.bytes, &entities).unwrap();
    let mut textures = catalog.textures().to_vec();
    let mut bindings = catalog.bindings().to_vec();
    let good_binding = bindings[0].clone();
    for field in 0..7 {
        bindings[0] = good_binding.clone();
        match field {
            0 => bindings[0].rig = u32::MAX,
            1 => bindings[0].entity_symbol = u32::MAX,
            2 => bindings[0].geometry = u32::MAX,
            3 => bindings[0].render_controller = u32::MAX,
            4 => bindings[0].texture = u32::MAX,
            5 => bindings[0].geometry_candidate = u32::MAX,
            _ => bindings[0].material = "unreviewed".into(),
        }
        assert!(encode_actor_catalog(&entities, &textures, &bindings).is_err());
    }
    bindings[0] = good_binding;
    bindings.push(bindings[0].clone());
    assert!(encode_actor_catalog(&entities, &textures, &bindings).is_err());
    bindings.pop();
    textures[0].source = u32::MAX;
    assert!(encode_actor_catalog(&entities, &textures, &bindings).is_err());
    // Valid outer digest and pixel digest cannot authorize fractional alpha.
    let mut malicious = compiled.bytes;
    malicious[128 + 40 + 3] = 128;
    let pixel_end = 128 + 40 + 16 * 16 * 4;
    let pixel_hash = Sha256::digest(&malicious[168..pixel_end]);
    malicious[136..168].copy_from_slice(&pixel_hash);
    let end = malicious.len() - 32;
    let outer_hash = Sha256::digest(&malicious[..end]);
    malicious[end..].copy_from_slice(&outer_hash);
    assert!(RuntimeActorCatalog::decode(&malicious, &entities).is_err());
}

#[test]
fn sibling_compiler_rejects_linked_texture_directory_outside_pack() {
    let pack = pack(0, "entity_alphatest", false);
    let outside = tempfile::tempdir().unwrap();
    let link = pack.path().join("textures/entity");
    let target = outside.path().join("entity");
    fs::rename(&link, &target).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).unwrap();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let link_arg = link.to_str().unwrap().replace('/', "\\");
        let target_arg = target.to_str().unwrap().replace('/', "\\");
        assert!(
            std::process::Command::new("cmd")
                .args(["/d", "/c"])
                .raw_arg(format!("mklink /J \"{link_arg}\" \"{target_arg}\""))
                .status()
                .unwrap()
                .success()
        );
    }
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(&target).unwrap()
    );
    assert!(compile_actor_assets(pack.path(), MANIFEST).is_err());
}

#[test]
fn explicit_negative_uv_sizes_admit_only_both_in_bounds_endpoints() {
    for (origin, size, expected) in [(16, -16, 1), (0, 16, 1), (17, -16, 0), (0, -1, 0)] {
        let pack = pack(0, "entity_alphatest", false);
        write(pack.path(), "models/entity/example.geo.json", format!(r#"{{"format_version":"1.12.0","minecraft:geometry":[{{"description":{{"identifier":"geometry.example","texture_width":16,"texture_height":16}},"bones":[{{"name":"root","cubes":[{{"origin":[0,0,0],"size":[0,2,7],"uv":{{"west":{{"uv":[{origin},0],"uv_size":[{size},2]}},"east":{{"uv":[0,0],"uv_size":[16,2]}}}}}}]}}]}}]}}"#).as_bytes());
        let compiled = compile_actor_assets(pack.path(), MANIFEST).unwrap();
        assert_eq!(compiled.report.bindings, expected);
        if expected == 0 {
            assert!(
                compiled
                    .report
                    .fallbacks
                    .iter()
                    .any(|fallback| fallback.reason.as_ref() == "uv_extents_or_inheritance")
            );
        }
    }
}

#[test]
fn sibling_compiler_preserves_duplicate_json_and_source_size_protections() {
    let duplicate = pack(0, "entity_alphatest", false);
    write(
        duplicate.path(),
        "render_controllers/example.json",
        br#"{"render_controllers":{},"render_controllers":{}}"#,
    );
    assert!(compile_actor_assets(duplicate.path(), MANIFEST).is_err());
    let oversized = pack(0, "entity_alphatest", false);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(oversized.path().join("textures/entity/example.png"))
        .unwrap();
    file.set_len(assets::MAX_ENTITY_SOURCE_BYTES as u64 + 1)
        .unwrap();
    assert!(compile_actor_assets(oversized.path(), MANIFEST).is_err());
    let escaped = pack(0, "entity_alphatest", false);
    write(escaped.path(), "entity/example.entity.json", br#"{"format_version":"1.8.0","minecraft:client_entity":{"description":{"identifier":"minecraft:example","textures":{"default":"../outside"}}}}"#);
    if let Ok(compiled) = compile_actor_assets(escaped.path(), MANIFEST) {
        assert_eq!(compiled.report.bindings, 0);
    }
}
