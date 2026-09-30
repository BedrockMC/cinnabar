#import bevy_render::view::View

struct AtmosphereUniform {
    sun_direction_daylight: vec4<f32>,
    moon_direction_phase: vec4<f32>,
    sky_zenith_rain: vec4<f32>,
    sky_horizon_thunder: vec4<f32>,
    fog_color_start: vec4<f32>,
    fog_end_time: vec4<f32>,
    sunrise_band: vec4<f32>,
    sky_extra: vec4<f32>,
}

struct PackedCloudQuad {
    bounds: u32,
    face_and_axis: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> atmosphere: AtmosphereUniform;
@group(0) @binding(2) var<storage, read> cloud_records: array<PackedCloudQuad>;

// Vanilla 26.30 cloud layer: one `clouds.png` texel spans 16x16 blocks, the slab is four
// blocks thick at 192.33, faces carry the tessellator's baked shade, and alpha fades by
// distance. There is no fog and no directional light.
const CLOUD_UNDERSIDE_Y: f32 = 192.33;
const CLOUD_TOP_Y: f32 = 196.33;
const CLOUD_CELL_BLOCKS: f32 = 16.0;
const CLOUD_TEXTURE_WORLD_PERIOD: f32 = 4096.0;
const FACE_DOWN: u32 = 0u;
const FACE_UP: u32 = 1u;
const FACE_NORTH: u32 = 2u;
const FACE_SOUTH: u32 = 3u;
const FACE_WEST: u32 = 4u;
const RAIN_CLOUD_COLOUR: vec3<f32> = vec3(191.0 / 255.0);
const THUNDER_CLOUD_COLOUR: vec3<f32> = vec3(30.0 / 255.0);
const WEATHER_COLOUR_CONTRIBUTION: f32 = 0.95;
const CLOUD_ALPHA: f32 = 0.7;
const CLOUD_FADE_START: f32 = 0.9;
const CLOUD_SUNRISE_WEIGHT: f32 = 0.35;
const TAU: f32 = 6.2831855;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) @interpolate(flat) normal: vec3<f32>,
}

fn face_normal(face: u32) -> vec3<f32> {
    if (face == FACE_DOWN) {
        return vec3(0.0, -1.0, 0.0);
    }
    if (face == FACE_UP) {
        return vec3(0.0, 1.0, 0.0);
    }
    if (face == FACE_NORTH) {
        return vec3(0.0, 0.0, -1.0);
    }
    if (face == FACE_SOUTH) {
        return vec3(0.0, 0.0, 1.0);
    }
    if (face == FACE_WEST) {
        return vec3(-1.0, 0.0, 0.0);
    }
    return vec3(1.0, 0.0, 0.0);
}

// Mirrors `atmosphere::cloud_face_shade`.
fn face_shade(normal: vec3<f32>) -> f32 {
    return clamp(
        0.55 * 0.5 * (normal.y + 1.0) - 0.1 * normal.x * normal.x + 0.1 * normal.z * normal.z + 0.75,
        0.0,
        1.0,
    );
}

// Mirrors `atmosphere::cloud_colour`.
fn cloud_colour() -> vec3<f32> {
    let rain_colour = mix(
        vec3(1.0),
        RAIN_CLOUD_COLOUR,
        clamp(atmosphere.sky_zenith_rain.w, 0.0, 1.0) * WEATHER_COLOUR_CONTRIBUTION,
    );
    let weather_colour = mix(
        rain_colour,
        THUNDER_CLOUD_COLOUR,
        clamp(atmosphere.sky_horizon_thunder.w, 0.0, 1.0) * WEATHER_COLOUR_CONTRIBUTION,
    );
    let brightness = clamp(2.0 * cos(TAU * atmosphere.sky_extra.y) + 0.5, 0.0, 1.0);
    let base = weather_colour * vec3(0.9 * brightness + 0.1, 0.9 * brightness + 0.1, 0.85 * brightness + 0.15);
    let weight = clamp(atmosphere.sunrise_band.w, 0.0, 1.0) * CLOUD_SUNRISE_WEIGHT;
    return max(atmosphere.sunrise_band.rgb * weight + base * (1.0 - weight), vec3(0.0));
}

// Mirrors `atmosphere::cloud_distance_fade`.
fn distance_fade(world_distance: f32) -> f32 {
    let fade_distance = atmosphere.fog_end_time.w;
    if (fade_distance <= 0.0) {
        return 1.0;
    }
    return clamp(1.0 - max(world_distance / fade_distance - CLOUD_FADE_START, 0.0), 0.0, 1.0);
}

fn corner_uv(corner_index: u32) -> vec2<f32> {
    return array<vec2<f32>, 6>(
        vec2(0.0, 0.0),
        vec2(1.0, 0.0),
        vec2(1.0, 1.0),
        vec2(0.0, 0.0),
        vec2(1.0, 1.0),
        vec2(0.0, 1.0),
    )[corner_index];
}

fn reconstruct_local_position(record: PackedCloudQuad, corner: vec2<f32>) -> vec3<f32> {
    let axis0_start = f32(record.bounds & 0xffu);
    let axis1_start = f32((record.bounds >> 8u) & 0xffu);
    let axis0_extent = f32(((record.bounds >> 16u) & 0xffu) + 1u);
    let axis1_extent = f32(((record.bounds >> 24u) & 0xffu) + 1u);
    let face = record.face_and_axis & 0x7u;

    let x = mix(axis0_start, axis0_start + axis0_extent, corner.x);
    let z = mix(axis1_start, axis1_start + axis1_extent, corner.y);
    if (face == FACE_DOWN) {
        return vec3(x, CLOUD_UNDERSIDE_Y, z);
    }
    if (face == FACE_UP) {
        return vec3(x, CLOUD_TOP_Y, z);
    }

    let run = mix(axis0_start, axis0_start + axis0_extent, corner.x);
    let y = mix(CLOUD_UNDERSIDE_Y, CLOUD_TOP_Y, corner.y);
    if (face == FACE_NORTH) {
        return vec3(run, y, axis1_start);
    }
    if (face == FACE_SOUTH) {
        return vec3(run, y, axis1_start);
    }
    if (face == FACE_WEST) {
        return vec3(axis1_start, y, run);
    }
    return vec3(axis1_start, y, run);
}

@vertex
fn cloud_vertex(
    @builtin(vertex_index) vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
) -> VertexOutput {
    let quad_index = vertex_index / 6u;
    let corner_index = vertex_index % 6u;
    let record = cloud_records[quad_index];
    let local_position = reconstruct_local_position(record, corner_uv(corner_index));

    let cloud_texture_offset = atmosphere.fog_end_time.z * CLOUD_TEXTURE_WORLD_PERIOD;
    let center_x = floor((view.world_position.x - cloud_texture_offset) / CLOUD_TEXTURE_WORLD_PERIOD)
        * CLOUD_TEXTURE_WORLD_PERIOD + cloud_texture_offset;
    let center_z = floor(view.world_position.z / CLOUD_TEXTURE_WORLD_PERIOD)
        * CLOUD_TEXTURE_WORLD_PERIOD;
    let instance_column = i32(instance_index % 3u) - 1;
    let instance_row = i32(instance_index / 3u) - 1;
    let instance_origin = vec2(
        center_x + f32(instance_column) * CLOUD_TEXTURE_WORLD_PERIOD,
        center_z + f32(instance_row) * CLOUD_TEXTURE_WORLD_PERIOD,
    );
    let world_position = vec3(
        local_position.x * CLOUD_CELL_BLOCKS + instance_origin.x,
        local_position.y,
        local_position.z * CLOUD_CELL_BLOCKS + instance_origin.y,
    );

    var out: VertexOutput;
    out.position = view.clip_from_world * vec4(world_position, 1.0);
    out.world_position = world_position;
    out.normal = face_normal(record.face_and_axis & 0x7u);
    return out;
}

@fragment
fn cloud_fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let colour = cloud_colour() * face_shade(in.normal);
    let alpha = CLOUD_ALPHA * distance_fade(distance(in.world_position, view.world_position));
    return vec4(colour, alpha);
}
