#[path = "support/shader_source.rs"]
mod shader_source;

/// Resolve the unchanged vanilla shader for standalone validation.
fn standalone(source: &str) -> String {
    shader_source::standalone(source, &[])
}

#[test]
fn every_shader_parses_and_validates() {
    let mut failures = Vec::new();
    let mut validated = 0;
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".wgsl") || name == "lighting.wgsl" || name == "biome_tint.wgsl" {
            continue;
        }
        let source = standalone(&std::fs::read_to_string(&path).unwrap());
        match naga::front::wgsl::parse_str(&source) {
            Err(error) => failures.push(format!("{name}: {error}")),
            Ok(module) => {
                if let Err(error) = naga::valid::Validator::new(
                    naga::valid::ValidationFlags::all(),
                    naga::valid::Capabilities::all(),
                )
                .validate(&module)
                {
                    failures.push(format!("{name}: {error:?}"));
                }
            }
        }
        validated += 1;
    }
    assert!(validated >= 10, "shader sources were not found");
    assert!(failures.is_empty(), "{failures:#?}");
}
