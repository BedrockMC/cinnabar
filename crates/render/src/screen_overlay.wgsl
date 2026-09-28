// Full-screen camera overlays composited back to front in one pass.
// Alphas, patterns and the procedural fallbacks are provisional and need native measurement.

struct Layer {
    // rgb tint, alpha
    color: vec4<f32>,
    // kind, unused x3
    params: vec4<f32>,
}

struct Overlays {
    // layer count, clock seconds, textures present (0/1), unused
    header: vec4<f32>,
    layers: array<Layer, 8>,
}

@group(0) @binding(0) var<uniform> overlays: Overlays;
@group(0) @binding(1) var overlay_textures: texture_2d_array<f32>;
@group(0) @binding(2) var overlay_sampler: sampler;

const KIND_POWDER_SNOW: u32 = 1u;
const KIND_FIRE: u32 = 2u;
const KIND_PORTAL: u32 = 3u;
const KIND_PUMPKIN: u32 = 4u;
const KIND_SPYGLASS: u32 = 5u;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn overlay_vertex(@builtin(vertex_index) index: u32) -> VertexOutput {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let cell = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash(cell);
    let b = hash(cell + vec2<f32>(1.0, 0.0));
    let c = hash(cell + vec2<f32>(0.0, 1.0));
    let d = hash(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Returns rgb and coverage for one layer before its own alpha scale.
fn shade(kind: u32, tint: vec3<f32>, uv: vec2<f32>, aspect: f32, clock: f32, textured: bool) -> vec4<f32> {
    let centred = uv - vec2<f32>(0.5);
    let round_r = length(centred * vec2<f32>(aspect, 1.0));
    var result = vec4<f32>(tint, 1.0);
    if kind == KIND_POWDER_SNOW {
        result = vec4<f32>(tint, smoothstep(0.3, 0.75, round_r * 1.4));
    } else if kind == KIND_FIRE {
        let flicker = value_noise(vec2<f32>(uv.x * 9.0, clock * 6.0));
        let height = smoothstep(0.55 - 0.12 * flicker, 1.0, uv.y);
        let flame = mix(vec3<f32>(1.0, 0.55, 0.1), vec3<f32>(1.0, 0.85, 0.3), uv.y);
        result = vec4<f32>(flame * tint, height);
    } else if kind == KIND_PORTAL {
        let swirl = value_noise(uv * vec2<f32>(aspect, 1.0) * 6.0 + vec2<f32>(clock * 0.4, -clock * 0.3));
        let pulse = 0.55 + 0.45 * sin(clock * 2.0 + swirl * 6.0);
        result = vec4<f32>(vec3<f32>(0.45, 0.2, 0.85) * tint, 0.6 + 0.4 * pulse * swirl);
    } else if kind == KIND_PUMPKIN {
        let texel = textureSampleLevel(overlay_textures, overlay_sampler, uv, 0, 0.0);
        let procedural = vec4<f32>(0.04, 0.02, 0.0, smoothstep(0.35, 0.8, round_r));
        result = select(procedural, vec4<f32>(texel.rgb * tint, texel.a), textured);
    } else if kind == KIND_SPYGLASS {
        // A square scope sized to the view height, centred, with black side bars.
        let scope = vec2<f32>(centred.x * aspect + 0.5, uv.y);
        let inside = scope.x >= 0.0 && scope.x <= 1.0;
        let texel = textureSampleLevel(overlay_textures, overlay_sampler, scope, 1, 0.0);
        let procedural = vec4<f32>(0.0, 0.0, 0.0, smoothstep(0.46, 0.5, length(scope - vec2<f32>(0.5))));
        let scoped = select(procedural, vec4<f32>(texel.rgb * tint, texel.a), textured);
        result = select(vec4<f32>(0.0, 0.0, 0.0, 1.0), scoped, inside);
    }
    return result;
}

@fragment
fn overlay_fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let derivative = fwidth(in.uv);
    let aspect = max(derivative.y / max(derivative.x, 1.0e-8), 1.0e-3);
    let count = u32(overlays.header.x);
    let clock = overlays.header.y;
    let textured = overlays.header.z > 0.5;
    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        if i >= count {
            break;
        }
        let layer = overlays.layers[i];
        let shaded = shade(u32(layer.params.x), layer.color.rgb, in.uv, aspect, clock, textured);
        let src_alpha = clamp(shaded.a * layer.color.a, 0.0, 1.0);
        let out_alpha = src_alpha + alpha * (1.0 - src_alpha);
        rgb = (shaded.rgb * src_alpha + rgb * alpha * (1.0 - src_alpha)) / max(out_alpha, 1.0e-5);
        alpha = out_alpha;
    }
    return vec4<f32>(rgb, alpha);
}
