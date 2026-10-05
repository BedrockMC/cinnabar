use std::io::Write;

use serde_json::json;

fn archive(id: u128, material: serde_json::Value) -> protocol::ResourcePackArchive {
    let id = format!("00000000-0000-0000-0000-{id:012x}");
    let manifest = json!({"format_version":2,"header":{"uuid":id,"version":[1,0,0]},
        "modules":[{"type":"resources"}]});
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, value) in [
        ("manifest.json", manifest),
        ("materials/entity.material", material),
    ] {
        zip.start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&value).unwrap()).unwrap();
    }
    protocol::ResourcePackArchive::unencrypted(
        id.parse().unwrap(),
        "1.0.0".into(),
        String::new(),
        zip.finish().unwrap().into_inner(),
    )
}

#[test]
fn actor_material_collector_retains_custom_inheritance_and_highest_layer_child() {
    let lower = archive(
        1,
        json!({"materials":{
            "version":"1.0.0",
            "fixture:entity_alphatest":{"+states":["Blending"]},
            "unchanged:entity_alphatest":{"-defines":["FANCY"]}
        }}),
    );
    let upper = archive(
        2,
        json!({"materials":{
            "version":"1.0.0",
            "fixture:entity_alphatest_one_sided":{"+states":["DisableDepthWrite"]}
        }}),
    );
    let view = resource_pack::LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(vec![lower, upper]),
    ));
    let files = super::collect_files(&view, None);
    let definitions = files
        .iter()
        .find_map(|(path, bytes)| {
            path.starts_with("materials/").then(|| {
                serde_json::from_slice::<serde_json::Value>(bytes).unwrap()["materials"].clone()
            })
        })
        .expect("authored material definitions must reach entity compilation");
    assert_eq!(
        definitions["fixture:entity_alphatest_one_sided"],
        json!({"+states":["DisableDepthWrite"]})
    );
    assert_eq!(
        definitions["unchanged:entity_alphatest"],
        json!({"-defines":["FANCY"]})
    );
    assert!(definitions.get("fixture:entity_alphatest").is_none());
}
