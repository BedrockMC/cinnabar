use crate::chunk::*;

pub(in crate::chunk) fn packed_lighting_records(lighting: &[PackedQuadLighting]) -> Vec<[u16; 4]> {
    lighting
        .iter()
        .copied()
        .map(PackedQuadLighting::samples)
        .collect()
}
