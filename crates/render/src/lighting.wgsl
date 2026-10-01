#define_import_path cinnabar::lighting

@group(1) @binding(0) var<uniform> world_lightmap: array<vec4<f32>, 256>;

// Both nibbles address the same environment table in every ordinary world pass.
fn light_colour(sample: u32) -> vec3<f32> {
    return world_lightmap[sample & 255u].rgb;
}

// Kept separate from the lightmap: AO shades geometry, not light coordinates.
fn light_ao_factor(level: u32) -> f32 {
    return 1.0 - f32(min(level, 3u)) * 0.12;
}

// Vertex colors interpolate the lightmap result rather than nonlinear light levels.
fn lit_colour(colour: vec3<f32>, lighting: vec3<f32>) -> vec3<f32> {
    return colour * lighting;
}
