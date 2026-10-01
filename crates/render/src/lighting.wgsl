#define_import_path cinnabar::lighting

@group(1) @binding(0) var<uniform> world_lightmap: array<vec4<f32>, 256>;

// Both nibbles address the same environment table in every ordinary world pass.
fn light_colour(sample: u32) -> vec3<f32> {
    return world_lightmap[sample & 255u].rgb;
}

// Kept separate from the lightmap: AO shades geometry, not light coordinates.
fn light_ao_factor(level: u32) -> f32 {
    return 1.0 - f32(min(level, 4u)) * 0.2;
}

// Vertex colors interpolate the lightmap result rather than nonlinear light levels.
fn lit_colour(colour: vec3<f32>, lighting: vec3<f32>) -> vec3<f32> {
    return colour * lighting;
}

// Lens 1.26.50.26 0x6a07d80: ordinary and emitting face coefficients.
fn face_shade(normal: vec3<f32>, emitting: bool) -> f32 {
    if normal.y < -0.5 { return select(0.5, 0.875, emitting); }
    if abs(normal.x) > 0.5 { return select(0.6, 0.9, emitting); }
    if abs(normal.z) > 0.5 { return select(0.8, 0.95, emitting); }
    return 1.0;
}
