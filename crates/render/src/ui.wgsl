struct UiViewport {
    viewport_size: vec2<f32>,
    time_seconds: f32,
    glint_strength: f32,
};

// Vertex style bit for the enchantment glint (`ui::UI_STYLE_GLINT`).
const STYLE_GLINT: u32 = 2u;

@group(0) @binding(0) var<uniform> viewport: UiViewport;
@group(0) @binding(1) var ui_pages: texture_2d_array<f32>;
@group(0) @binding(2) var ui_sampler: sampler;

struct UiVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) texture_page: u32,
    @location(3) @interpolate(flat) style_flags: u32,
};

// Vertex colours are authored in sRGB (JSON-UI and HUD colours alike); the
// target stores linear values, so decode them as the texture pages are.
fn srgb_to_linear(srgb: vec3<f32>) -> vec3<f32> {
    let low = srgb / 12.92;
    let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, srgb <= vec3<f32>(0.04045));
}

@vertex
fn ui_vertex(
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<u32>,
    @location(2) color: vec4<f32>,
    @location(3) style_flags: u32,
    @builtin(instance_index) texture_page: u32,
) -> UiVertexOutput {
    let ndc = vec2<f32>(
        position.x / viewport.viewport_size.x * 2.0 - 1.0,
        1.0 - position.y / viewport.viewport_size.y * 2.0,
    );
    var output: UiVertexOutput;
    output.clip_position = vec4<f32>(ndc, 0.0, 1.0);
    output.uv = vec2<f32>(uv);
    output.color = vec4<f32>(srgb_to_linear(color.rgb), color.a);
    output.texture_page = texture_page;
    output.style_flags = style_flags;
    return output;
}

@fragment
fn ui_fragment(input: UiVertexOutput) -> @location(0) vec4<f32> {
    let dimensions = vec2<f32>(textureDimensions(ui_pages));
    // Vertex UVs address texel *edges*: a glyph spans x0..x0+width. Linear
    // interpolation across the quad therefore already lands on texel centres,
    // and adding half a texel here shifted the whole nearest-sampling grid by
    // half a texel. At a 1:1 draw that sampled one texel to the right; at a 2x
    // draw it gave the leading column one pixel, every other column two, and
    // bled a column of the neighbouring glyph in on the right.
    let normalized_uv = input.uv / dimensions;
    let sample = textureSample(
        ui_pages,
        ui_sampler,
        normalized_uv,
        i32(input.texture_page),
    );
    let straight_color = input.color;
    let alpha = sample.a * straight_color.a;
    var premultiplied_rgb = sample.rgb * sample.a * straight_color.rgb * straight_color.a;
    if (input.style_flags & STYLE_GLINT) != 0u {
        // L:1.26.50.26:0x213ce90 scales glint RGB without changing alpha.
        premultiplied_rgb += glint(input.clip_position.xy) * viewport.glint_strength * alpha;
    }
    return vec4<f32>(premultiplied_rgb, alpha);
}

// Two diagonal purple bands scrolling at different rates, added over opaque texels like the
// item glint layers. Provisional: procedural, not the retail glint texture.
fn glint(pixel: vec2<f32>) -> vec3<f32> {
    let t = viewport.time_seconds;
    let a = fract((pixel.x * 0.96 + pixel.y * 0.28) / 96.0 - t * 0.33);
    let b = fract((pixel.x * 0.5 - pixel.y * 0.87) / 80.0 + t * 0.21);
    return vec3<f32>(0.5, 0.25, 0.8) * (glint_band(a) + glint_band(b)) * 0.55;
}

fn glint_band(x: f32) -> f32 {
    return smoothstep(0.0, 0.18, x) * (1.0 - smoothstep(0.18, 0.42, x));
}
