#define_import_path cinnabar::biome_tint

// Bedrock blends over a 4-block lattice: each lattice point averages biomes
// sampled at its own position and one lattice step either side, and queries
// interpolate linearly between the two nearest lattice points. Per axis that
// collapses to four taps at lattice multiples with weights (1-t, 1, 1, t) / 3.
// The vertical lattice axis is not blended here (needs measurement).
const BIOME_LATTICE_STEP: i32 = 4;
const BIOME_DESCRIPTOR_WORDS: u32 = 11u;
const BIOME_DESCRIPTOR_MAGIC: u32 = 0x42494f31u;

struct BiomeTintGpu {
    grass: u32,
    foliage: u32,
    birch: u32,
    evergreen: u32,
    dry_foliage: u32,
    water: u32,
    flags: u32,
    padding: u32,
}

@group(0) @binding(7) var<storage, read> biome_records: array<u32>;
@group(0) @binding(8) var<storage, read> biome_tints: array<BiomeTintGpu>;

fn unpack_linear_rgb10(packed: u32) -> vec3<f32> {
    return vec3<f32>(
        f32(packed & 0x3ffu),
        f32((packed >> 10u) & 0x3ffu),
        f32((packed >> 20u) & 0x3ffu),
    ) / 1023.0;
}

fn packed_payload_tint_index(payload: u32, coordinate: vec3<u32>) -> u32 {
    let header = biome_records[payload];
    let bits = header & 0xffu;
    let palette_len = (header >> 8u) & 0x1fffu;
    if (palette_len == 0u) {
        return 0u;
    }
    var word_count = 0u;
    var palette_index = 0u;
    if (bits != 0u) {
        let per_word = 32u / bits;
        word_count = (4096u + per_word - 1u) / per_word;
        let linear = (coordinate.x << 8u) | (coordinate.z << 4u) | coordinate.y;
        let word = biome_records[payload + 1u + linear / per_word];
        palette_index = (word >> ((linear % per_word) * bits)) & ((1u << bits) - 1u);
    }
    if (palette_index >= palette_len) {
        return 0u;
    }
    return biome_records[payload + 1u + word_count + palette_index];
}

fn packed_biome_tint_index(record: u32, source_coordinate: vec3<i32>) -> u32 {
    if (biome_records[record] != BIOME_DESCRIPTOR_MAGIC) {
        return 0u;
    }
    let dx = source_coordinate.x >> 4;
    let dz = source_coordinate.z >> 4;
    if (dx < -1 || dx > 1 || dz < -1 || dz > 1) {
        return 0u;
    }
    let slot = u32((dz + 1) * 3 + dx + 1);
    var relative = biome_records[record + 2u + slot];
    var coordinate = source_coordinate;
    if (relative == 0u) {
        relative = biome_records[record + 2u + 4u];
        coordinate.x = clamp(coordinate.x, 0, 15);
        coordinate.z = clamp(coordinate.z, 0, 15);
    } else {
        coordinate.x -= dx * 16;
        coordinate.z -= dz * 16;
    }
    coordinate.y = clamp(coordinate.y, 0, 15);
    return packed_payload_tint_index(record + relative, vec3<u32>(coordinate));
}

fn safe_biome_tint(index: u32) -> BiomeTintGpu {
    let safe_index = select(0u, index, index < arrayLength(&biome_tints));
    return biome_tints[safe_index];
}

fn tint_domain_colour(tint: BiomeTintGpu, tint_kind: u32) -> vec3<f32> {
    if (tint_kind == 0x10u) {
        return unpack_linear_rgb10(tint.grass);
    }
    if (tint_kind == 0x30u) {
        return unpack_linear_rgb10(tint.water);
    }
    return unpack_linear_rgb10(tint.foliage);
}

fn special_foliage_tint(tint: BiomeTintGpu, material_flags: u32) -> vec3<f32> {
    switch material_flags & 0x600u {
        case 0x200u: { return unpack_linear_rgb10(tint.birch); }
        case 0x400u: { return unpack_linear_rgb10(tint.evergreen); }
        case 0x600u: { return unpack_linear_rgb10(tint.dry_foliage); }
        default: { return unpack_linear_rgb10(tint.foliage); }
    }
}

fn lattice_tap_weights(t: f32) -> vec4<f32> {
    return vec4((1.0 - t) / 3.0, 1.0 / 3.0, 1.0 / 3.0, t / 3.0);
}

fn blended_biome_tint(
    tint_kind: u32,
    material_flags: u32,
    record: u32,
    local_position: vec3<f32>,
) -> vec3<f32> {
    let coordinate = vec3<i32>(floor(local_position));
    if (tint_kind == 0x20u && (material_flags & 0x600u) != 0u) {
        return special_foliage_tint(
            safe_biome_tint(packed_biome_tint_index(record, coordinate)),
            material_flags,
        );
    }

    let uniform_tint = biome_records[record + 1u];
    if (uniform_tint != 0xffffffffu) {
        return tint_domain_colour(safe_biome_tint(uniform_tint), tint_kind);
    }

    let cell = vec2<i32>(coordinate.x >> 2u, coordinate.z >> 2u);
    let fraction = vec2<f32>(
        f32(coordinate.x & 3),
        f32(coordinate.z & 3),
    ) / f32(BIOME_LATTICE_STEP);
    let weights_x = lattice_tap_weights(fraction.x);
    let weights_z = lattice_tap_weights(fraction.y);
    var sum = vec3(0.0);
    for (var tz = 0; tz < 4; tz += 1) {
        for (var tx = 0; tx < 4; tx += 1) {
            let sample_coordinate = vec3(
                (cell.x - 1 + tx) * BIOME_LATTICE_STEP,
                coordinate.y,
                (cell.y - 1 + tz) * BIOME_LATTICE_STEP,
            );
            let tint_index = packed_biome_tint_index(record, sample_coordinate);
            sum += tint_domain_colour(safe_biome_tint(tint_index), tint_kind)
                * (weights_x[tx] * weights_z[tz]);
        }
    }
    return sum;
}
