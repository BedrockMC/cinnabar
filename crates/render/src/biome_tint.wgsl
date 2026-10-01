#define_import_path cinnabar::biome_tint

// The CPU packs vanilla's 27-sample biome counts at each 3D lattice point.
// BIOME_CONSTANTS

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
    let offset = source_coordinate >> vec3(4u);
    if (any(offset < vec3(-1)) || any(offset > vec3(1))) { return 0u; }
    let layer = select(select(2u, 1u, offset.y < 0), 0u, offset.y == 0);
    let slot = layer * 9u + u32((offset.z + 1) * 3 + offset.x + 1);
    let relative = biome_records[record + 2u + slot];
    if (relative == 0u) { return 0u; }
    return packed_payload_tint_index(record + relative, vec3<u32>(source_coordinate & vec3(15)));
}

fn safe_biome_tint(index: u32) -> BiomeTintGpu {
    let safe_index = select(0u, index, index < arrayLength(&biome_tints));
    return biome_tints[safe_index];
}

fn tint_domain_colour(tint: BiomeTintGpu, tint_kind: u32, material_flags: u32) -> vec3<f32> {
    if (tint_kind == 0x10u) {
        return unpack_linear_rgb10(tint.grass);
    }
    if (tint_kind == 0x30u) {
        return unpack_linear_rgb10(tint.water);
    }
    return special_foliage_tint(tint, material_flags);
}

fn special_foliage_tint(tint: BiomeTintGpu, material_flags: u32) -> vec3<f32> {
    switch material_flags & 0x600u {
        case 0x200u: { return unpack_linear_rgb10(tint.birch); }
        case 0x400u: { return unpack_linear_rgb10(tint.evergreen); }
        case 0x600u: { return unpack_linear_rgb10(tint.dry_foliage); }
        default: { return unpack_linear_rgb10(tint.foliage); }
    }
}

// Tint tables are linear; vanilla averages normalized palette RGB before lighting.
fn tint_to_gamma(linear: vec3<f32>) -> vec3<f32> {
    return select(12.92 * linear, 1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055, linear > vec3(0.0031308));
}

fn tint_to_linear(gamma: vec3<f32>) -> vec3<f32> {
    return select(gamma / 12.92, pow((gamma + 0.055) / 1.055, vec3(2.4)), gamma > vec3(0.04045));
}

fn lattice_point_index(position: vec3<i32>) -> u32 {
    let axis = vec3<u32>((position + vec3(BIOME_LATTICE_STEP)) / BIOME_LATTICE_STEP);
    return (axis.x * BIOME_LATTICE_SIDE + axis.y) * BIOME_LATTICE_SIDE + axis.z;
}

fn blended_biome_tint(
    tint_kind: u32,
    material_flags: u32,
    record: u32,
    local_position: vec3<f32>,
) -> vec3<f32> {
    let coordinate = vec3<i32>(floor(local_position));
    let uniform_tint = biome_records[record + 1u];
    if (uniform_tint != 0xffffffffu) {
        return tint_domain_colour(safe_biome_tint(uniform_tint), tint_kind, material_flags);
    }
    let base = (coordinate - vec3(BIOME_CACHE_ORIGIN)) / BIOME_LATTICE_STEP * BIOME_LATTICE_STEP + vec3(BIOME_CACHE_ORIGIN);
    let residue = vec3<u32>(coordinate - base + vec3(BIOME_RESIDUE_RADIUS));
    let query = ((residue.x * BIOME_RESIDUE_SIDE + residue.y) * BIOME_RESIDUE_SIDE + residue.z) * BIOME_QUERY_POINTS;
    var sum = vec3(0.0);
    var denominator = 0.0;
    for (var point = 0; point < i32(BIOME_QUERY_POINTS); point += 1) {
        let sample = BIOME_POINTS[query + u32(point)];
        let weight = sample.w;
        let position = base + vec3<i32>(sample.xyz);
        let start = record + BIOME_DESCRIPTOR_WORDS + lattice_point_index(position) * BIOME_POINT_WORDS;
        var colour = vec3(0.0);
        for (var i = 0u; i < biome_records[start]; i += 1u) {
            let tint = safe_biome_tint(biome_records[start + 1u + i]);
            colour += tint_to_gamma(tint_domain_colour(tint, tint_kind, material_flags)) * bitcast<f32>(biome_records[start + 1u + BIOME_BIOME_LIMIT + i]);
        }
        sum += clamp(colour, vec3(0.0), vec3(1.0)) * weight;
        denominator += weight;
    }
    return tint_to_linear(sum / denominator);
}
