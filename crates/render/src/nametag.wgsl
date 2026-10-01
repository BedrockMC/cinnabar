#import bevy_render::view::View

struct Record {
    anchor: vec3<f32>,
    text: u32,
    rect: vec4<f32>,
    uv: vec4<f32>,
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> records: array<Record>;
@group(0) @binding(2) var atlas: texture_2d<f32>;
@group(0) @binding(3) var atlas_sampler: sampler;

// World size of one font pixel and the text's lift toward the camera; mirror `nametag.rs`.
const BLOCKS_PER_FONT_PIXEL: f32 = 0.026666667;
const TEXT_LIFT_BLOCKS: f32 = 0.01;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) color: vec4<f32>,
    @location(2) @interpolate(flat) textured: u32,
}

fn corner(index: u32) -> vec2<f32> {
    return array<vec2<f32>, 6>(
        vec2(0.0, 0.0),
        vec2(1.0, 0.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(1.0, 1.0),
        vec2(0.0, 1.0),
    )[index];
}

@vertex
fn nametag_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let record = records[vertex_index / 6u];
    let at = corner(vertex_index % 6u);
    // The tag turns by yaw then pitch to face the camera position, with no roll.
    var facing = view.world_position - record.anchor;
    if (dot(facing, facing) < 1.0e-8) {
        facing = vec3(0.0, 0.0, 1.0);
    }
    facing = normalize(facing);
    var right = cross(vec3(0.0, 1.0, 0.0), facing);
    if (dot(right, right) < 1.0e-8) {
        right = vec3(1.0, 0.0, 0.0);
    }
    right = normalize(right);
    let up = cross(facing, right);
    let local = mix(record.rect.xy, record.rect.zw, at);
    var world = record.anchor + (right * local.x - up * local.y) * BLOCKS_PER_FONT_PIXEL;
    if (record.text != 0u) {
        world += facing * TEXT_LIFT_BLOCKS;
    }
    var out: VertexOutput;
    out.position = view.clip_from_world * vec4(world, 1.0);
    out.uv = mix(record.uv.xy, record.uv.zw, at);
    out.color = record.color;
    out.textured = select(0u, 1u, record.uv.z >= 0.0);
    return out;
}

@fragment
fn nametag_fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    var color = input.color;
    let texel = textureSample(atlas, atlas_sampler, input.uv);
    if (input.textured != 0u) {
        color = color * texel;
    }
    if (color.a <= 0.0) {
        discard;
    }
    return color;
}
