#define_import_path cinnabar::material

struct MaterialGpu {
    texture: u32,
    flags: u32,
    animation: u32,
    variation_start: u32,
    variation_count: u32,
    variation_weight: u32,
}

@group(0) @binding(3) var<storage, read> materials: array<MaterialGpu>;

// Lens 1.26.50.26 0x6a02170: wrapping coordinates, subtractive weights, last fallback.
fn positional_material(id: u32, position: vec3<i32>) -> MaterialGpu {
    let base = materials[id];
    if (base.variation_count == 0u) { return base; }
    if (base.variation_count == 1u) { return materials[base.variation_start]; }
    let p = vec3<u32>(position);
    let h = (p.z * 0x06ebfff5u) ^ (p.x * 0x002fc20fu) ^ p.y;
    var sample = f32(((h * 0x0285b825u + 11u) * h) >> 16u) / 65535.0;
    for (var i = 0u; i < base.variation_count; i += 1u) {
        let candidate = materials[base.variation_start + i];
        let weight = bitcast<f32>(candidate.variation_weight);
        if (sample <= weight || i + 1u == base.variation_count) { return candidate; }
        sample -= weight;
    }
    return base;
}
