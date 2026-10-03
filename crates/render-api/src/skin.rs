//! Shared skin layout limits and conversion, used by admission and rendering.

/// Local ceiling for the standard skin raster, including persona skins.
pub const MAX_STANDARD_SKIN_SIDE: u32 = 512;
/// Animated texture slots the vanilla player renderer adds to the base skin.
pub const MAX_SKIN_ANIMATION_LAYERS: usize = 3;

/// The authored square texture side for the classic skin layout.
pub const CLASSIC_SKIN_SIDE: usize = 64;
/// Largest square raster accepted by vanilla's classic skin validator.
pub const MAX_CLASSIC_SKIN_SIDE: usize = CLASSIC_SKIN_SIDE * 2;

/// Expands a legacy half-height skin to the square layout: the left limbs are the right limbs
/// with every face mirrored, as the legacy geometry draws them.
pub fn expand_legacy_skin_rgba8(rgba8: &[u8], side: usize) -> Vec<u8> {
    let scale = side / CLASSIC_SKIN_SIDE;
    let mut square = vec![0; side * side * 4];
    square[..rgba8.len()].copy_from_slice(rgba8);
    // (source x, source y, dest offset x, dest offset y, width, height) in 64-unit texels.
    const LIMB_FACES: [(usize, usize, isize, usize, usize, usize); 12] = [
        (4, 16, 16, 32, 4, 4),
        (8, 16, 16, 32, 4, 4),
        (0, 20, 24, 32, 4, 12),
        (4, 20, 16, 32, 4, 12),
        (8, 20, 8, 32, 4, 12),
        (12, 20, 16, 32, 4, 12),
        (44, 16, -8, 32, 4, 4),
        (48, 16, -8, 32, 4, 4),
        (40, 20, 0, 32, 4, 12),
        (44, 20, -8, 32, 4, 12),
        (48, 20, -16, 32, 4, 12),
        (52, 20, -8, 32, 4, 12),
    ];
    for (x, y, dx, dy, width, height) in LIMB_FACES {
        let (x, y, width, height) = (x * scale, y * scale, width * scale, height * scale);
        let target_x = (x as isize + dx * scale as isize) as usize;
        let target_y = y + dy * scale;
        for row in 0..height {
            for column in 0..width {
                let source = ((y + row) * side + x + column) * 4;
                let target = ((target_y + row) * side + target_x + width - 1 - column) * 4;
                let pixel: [u8; 4] = rgba8[source..source + 4].try_into().expect("four bytes");
                square[target..target + 4].copy_from_slice(&pixel);
            }
        }
    }
    square
}
