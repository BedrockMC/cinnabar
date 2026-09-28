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

// clock: animation seconds, sheet opacity, wind xz per block of fall.
// extent: blocks the sheet rises above the camera.
struct WeatherParams {
    clock: vec4<f32>,
    extent: vec4<f32>,
}

struct Column {
    x: i32,
    z: i32,
    bottom_y: f32,
    flags: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> atmosphere: AtmosphereUniform;
@group(0) @binding(2) var<storage, read> columns: array<Column>;
@group(0) @binding(3) var<uniform> weather: WeatherParams;

// Provisional look: streak and flake sizes and fall speeds need native measurement.
const RAIN_COLOUR: vec3<f32> = vec3(0.62, 0.72, 0.95);
const RAIN_FALL_BLOCKS_PER_SECOND: f32 = 22.0;
const RAIN_STREAKS_PER_BLOCK: f32 = 3.0;
const RAIN_STREAK_PERIOD: f32 = 2.2;
const SNOW_FALL_BLOCKS_PER_SECOND: f32 = 3.0;
const SNOW_FLAKES_PER_BLOCK: f32 = 4.0;
const SNOW_FLAKE_PERIOD: f32 = 1.2;
const NIGHT_PRECIPITATION_LIGHT: f32 = 0.3;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) pattern: vec2<f32>,
    @location(2) @interpolate(flat) seed: vec2<f32>,
    @location(3) @interpolate(flat) flags: u32,
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

fn hash11(value: f32) -> f32 {
    return fract(sin(value * 127.1) * 43758.5453);
}

@vertex
fn weather_vertex(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let column = columns[vertex_index / 6u];
    let corner = corner_uv(vertex_index % 6u);
    let centre = vec2(f32(column.x) + 0.5, f32(column.z) + 0.5);
    let to_centre = centre - view.world_position.xz;
    let facing = normalize(to_centre + vec2(0.0001, 0.0));
    let right = vec2(-facing.y, facing.x);
    let top = view.world_position.y + weather.extent.x;
    let y = mix(column.bottom_y, top, corner.y);
    let fall_height = y - column.bottom_y;
    let xz = centre + right * (corner.x - 0.5) + weather.clock.zw * fall_height;
    let world = vec3(xz.x, y, xz.y);

    var out: VertexOutput;
    out.position = view.clip_from_world * vec4(world, 1.0);
    out.world_position = world;
    out.pattern = vec2(corner.x, fall_height);
    out.seed = vec2(hash11(f32(column.x) * 0.37 + f32(column.z) * 1.91), hash11(f32(column.z) * 0.73 - f32(column.x) * 1.13));
    out.flags = column.flags;
    return out;
}

fn rain_coverage(pattern: vec2<f32>, seed: vec2<f32>, time: f32) -> f32 {
    let cell_position = pattern.x * RAIN_STREAKS_PER_BLOCK + seed.x * 17.0;
    let cell = floor(cell_position);
    let local = fract(cell_position);
    let jitter = hash11(cell + seed.y * 131.0);
    let across = 1.0 - smoothstep(0.045, 0.09, abs(local - (0.25 + 0.5 * jitter)));
    let fall = time * RAIN_FALL_BLOCKS_PER_SECOND * (0.85 + 0.3 * hash11(cell * 3.1 + seed.y));
    let phase = fract((pattern.y + fall + jitter * 7.0) / RAIN_STREAK_PERIOD);
    let dash = smoothstep(0.0, 0.08, phase) * (1.0 - smoothstep(0.35, 0.45, phase));
    return across * dash;
}

fn snow_coverage(pattern: vec2<f32>, seed: vec2<f32>, time: f32) -> f32 {
    let sway = sin(time * 1.3 + seed.x * 6.28318) * 0.15;
    let scaled = vec2(
        (pattern.x + sway) * SNOW_FLAKES_PER_BLOCK + seed.x * 13.0,
        (pattern.y + time * SNOW_FALL_BLOCKS_PER_SECOND) / SNOW_FLAKE_PERIOD + seed.y * 9.0,
    );
    let cell = floor(scaled);
    let local = fract(scaled);
    let key = cell.x * 1.7 + cell.y * 23.3 + seed.y * 5.0;
    if (hash11(key) > 0.55) {
        return 0.0;
    }
    let centre = vec2(0.25 + 0.5 * hash11(key + 1.0), 0.25 + 0.5 * hash11(key + 2.0));
    return 1.0 - smoothstep(0.10, 0.18, distance(local, centre));
}

@fragment
fn weather_fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let snow = (in.flags & 1u) != 0u;
    let column_alpha = f32((in.flags >> 8u) & 255u) / 255.0;
    let time = weather.clock.x;
    var coverage = rain_coverage(in.pattern, in.seed, time);
    var colour = RAIN_COLOUR;
    if (snow) {
        coverage = snow_coverage(in.pattern, in.seed, time) * 0.9;
        colour = vec3(1.0);
    }
    let light = mix(NIGHT_PRECIPITATION_LIGHT, 1.0, clamp(atmosphere.sun_direction_daylight.w, 0.0, 1.0));
    let fog = clamp(
        (distance(in.world_position, view.world_position) - atmosphere.fog_color_start.w)
            / max(atmosphere.fog_end_time.x - atmosphere.fog_color_start.w, 0.0001),
        0.0,
        1.0,
    );
    let alpha = coverage * column_alpha * weather.clock.y * (1.0 - fog);
    if (alpha <= 0.002) {
        discard;
    }
    return vec4(colour * light, alpha);
}
