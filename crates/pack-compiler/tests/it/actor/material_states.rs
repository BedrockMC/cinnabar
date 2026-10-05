use super::{MANIFEST, pack, write};
use serde_json::{Value, json};

fn state(material: &str, definitions: Option<&Value>) -> Value {
    let fixture = pack(0, material, false);
    if let Some(definitions) = definitions {
        write(
            fixture.path(),
            "materials/entity.material",
            &serde_json::to_vec(definitions).unwrap(),
        );
    }
    let compiled = pack_compiler::compile_entity_assets(fixture.path(), MANIFEST).unwrap();
    assert_eq!(compiled.render.layers.len(), 1);
    let bytes = assets::encode_entity_blob(&compiled).unwrap();
    let runtime = assets::RuntimeEntityAssets::decode(&bytes).unwrap();
    serde_json::to_value(runtime.render_data().layers[0]).unwrap()["material_state"].clone()
}

#[test]
fn actor_material_states_distinguish_authored_one_sided_and_nocull_alpha_test() {
    assert_eq!(
        state("entity_alphatest_one_sided", None),
        json!({"alpha_test":true,"cull":true,"blend":false,"depth_write":true})
    );
    assert_eq!(
        state("entity_alphatest", None),
        json!({"alpha_test":true,"cull":false,"blend":false,"depth_write":true})
    );
}

#[test]
fn actor_material_states_inherit_alpha_test_and_add_blending_without_losing_depth_write() {
    let definitions = json!({"materials":{
        "version":"1.0.0",
        "fixture_text:entity_alphatest":{"-defines":["FANCY"]},
        "fixture_plate:fixture_text":{"+states":["Blending"]}
    }});
    assert_eq!(
        state("fixture_plate", Some(&definitions)),
        json!({"alpha_test":true,"cull":false,"blend":true,"depth_write":true})
    );
}

#[test]
fn actor_material_states_apply_explicit_depth_and_culling_overrides_independently() {
    let definitions = json!({"materials":{
        "version":"1.0.0",
        "fixture_plate:entity_alphatest":{
            "+states":["Blending","DisableDepthWrite"],
            "-states":["DisableCulling"]
        },
        "fixture_opaque:fixture_plate":{
            "-states":["Blending"],"-defines":["ALPHA_TEST"]
        }
    }});
    assert_eq!(
        state("fixture_plate", Some(&definitions)),
        json!({"alpha_test":true,"cull":true,"blend":true,"depth_write":false})
    );
    assert_eq!(
        state("fixture_opaque", Some(&definitions)),
        json!({"alpha_test":false,"cull":true,"blend":false,"depth_write":false})
    );
}

#[test]
fn actor_material_states_replacement_excludes_add_remove_for_the_same_family() {
    let definitions = json!({"materials":{
        "version":"1.0.0",
        "fixture:entity_alphatest":{
            "states":["DisableDepthWrite"],
            "+states":["Blending"],
            "-states":["DisableDepthWrite"],
            "defines":["ALPHA_TEST"],
            "-defines":["ALPHA_TEST"]
        }
    }});
    assert_eq!(
        state("fixture", Some(&definitions)),
        json!({"alpha_test":true,"cull":true,"blend":false,"depth_write":false})
    );
}
