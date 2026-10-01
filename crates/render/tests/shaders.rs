//! Every shader validates, not just parses: naga's parser accepts colliding varying locations
//! and reserved identifiers that fail pipeline creation at runtime and silently skip the pass.
const VIEW: &str = "struct View { clip_from_world: mat4x4<f32>, unjittered_clip_from_world: mat4x4<f32>, \
view_from_world: mat4x4<f32>, world_from_view: mat4x4<f32>, clip_from_view: mat4x4<f32>, \
view_from_clip: mat4x4<f32>, world_position: vec3<f32>, exposure: f32, viewport: vec4<f32>, }";

fn standalone(source: &str) -> String {
    let lighting = include_str!("../src/lighting.wgsl").replacen(
        "#define_import_path cinnabar::lighting",
        "",
        1,
    );
    let biome_tint = meshing::biome_lattice::shader_source(include_str!("../src/biome_tint.wgsl"))
        .replacen("#define_import_path cinnabar::biome_tint", "", 1);
    source
        .replacen("#import bevy_render::view::View", VIEW, 1)
        .replacen(
            "#import cinnabar::lighting::{light_ao_factor, light_colour, lit_colour, face_shade}",
            &lighting,
            1,
        )
        .replacen(
            "#import cinnabar::lighting::{lit_colour, light_colour}",
            &lighting,
            1,
        )
        .replacen(
            "#import cinnabar::biome_tint::blended_biome_tint",
            &biome_tint,
            1,
        )
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
