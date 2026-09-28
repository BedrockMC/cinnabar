//! Lightning bolt geometry and flash envelope. Shape and timing constants are provisional
//! and need native calibration.

/// Seconds a strike lights the world.
pub const LIGHTNING_FLASH_SECONDS: f32 = 0.25;
/// Height of a bolt above its strike point.
pub const LIGHTNING_HEIGHT: f32 = 100.0;

const TRUNK_SEGMENTS: usize = 16;
const BRANCH_SEGMENTS: usize = 5;
const MAX_BRANCHES: usize = 3;
const STEP_JITTER: f32 = 3.0;

/// One straight piece of a bolt with the half-width it renders at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoltSegment {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub half_width: f32,
}

/// Flash brightness in `0..=1` at `age_seconds` after the strike; zero once it has faded.
#[must_use]
pub fn lightning_flash_level(age_seconds: f32) -> f32 {
    if !age_seconds.is_finite() || !(0.0..LIGHTNING_FLASH_SECONDS).contains(&age_seconds) {
        return 0.0;
    }
    1.0 - age_seconds / LIGHTNING_FLASH_SECONDS
}

/// A jagged trunk from the sky to `strike` plus a few shorter branches, fixed by `seed`.
#[must_use]
pub fn lightning_bolt_segments(seed: u64, strike: [f32; 3]) -> Vec<BoltSegment> {
    let mut rng = SplitMix(seed);
    let mut segments = Vec::with_capacity(TRUNK_SEGMENTS + MAX_BRANCHES * BRANCH_SEGMENTS);
    let step = LIGHTNING_HEIGHT / TRUNK_SEGMENTS as f32;
    // Walk down from the sky, then shift the whole trunk so it lands on the strike point.
    let mut points = Vec::with_capacity(TRUNK_SEGMENTS + 1);
    let mut offset = [0.0_f32; 2];
    points.push([0.0, LIGHTNING_HEIGHT, 0.0]);
    for index in 1..=TRUNK_SEGMENTS {
        offset[0] += rng.signed() * STEP_JITTER;
        offset[1] += rng.signed() * STEP_JITTER;
        points.push([offset[0], LIGHTNING_HEIGHT - step * index as f32, offset[1]]);
    }
    let landing = points[TRUNK_SEGMENTS];
    let shift = [strike[0] - landing[0], strike[1], strike[2] - landing[2]];
    for point in &mut points {
        *point = [
            point[0] + shift[0],
            point[1] + shift[1],
            point[2] + shift[2],
        ];
    }
    points[TRUNK_SEGMENTS] = strike;
    for pair in points.windows(2) {
        segments.push(BoltSegment {
            start: pair[0],
            end: pair[1],
            half_width: 0.35,
        });
    }
    for _ in 0..(rng.next() as usize % (MAX_BRANCHES + 1)) {
        let fork = 2 + rng.next() as usize % (TRUNK_SEGMENTS - 4);
        let mut from = points[fork];
        let drift = [rng.signed() * 4.0, rng.signed() * 4.0];
        for _ in 0..BRANCH_SEGMENTS {
            let to = [
                from[0] + drift[0] + rng.signed() * STEP_JITTER,
                from[1] - step,
                from[2] + drift[1] + rng.signed() * STEP_JITTER,
            ];
            segments.push(BoltSegment {
                start: from,
                end: to,
                half_width: 0.15,
            });
            from = to;
        }
    }
    segments
}

struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    /// Uniform value in `-1.0..1.0`.
    fn signed(&mut self) -> f32 {
        (self.next() >> 40) as f32 / 8_388_608.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_decays_linearly_and_ends() {
        assert_eq!(lightning_flash_level(0.0), 1.0);
        assert!((lightning_flash_level(LIGHTNING_FLASH_SECONDS * 0.5) - 0.5).abs() < 1.0e-6);
        assert_eq!(lightning_flash_level(LIGHTNING_FLASH_SECONDS), 0.0);
        assert_eq!(lightning_flash_level(-1.0), 0.0);
        assert_eq!(lightning_flash_level(f32::NAN), 0.0);
    }

    #[test]
    fn bolt_is_deterministic_and_lands_on_the_strike_point() {
        let strike = [10.0, 64.0, -5.0];
        let first = lightning_bolt_segments(42, strike);
        assert_eq!(first, lightning_bolt_segments(42, strike));
        assert_ne!(first, lightning_bolt_segments(43, strike));
        assert_eq!(first[TRUNK_SEGMENTS - 1].end, strike);
        assert!((first[0].start[1] - (64.0 + LIGHTNING_HEIGHT)).abs() < 1.0e-3);
    }

    #[test]
    fn bolt_segments_are_finite_and_branches_are_thinner() {
        for seed in 0..64 {
            let segments = lightning_bolt_segments(seed, [0.0, 0.0, 0.0]);
            assert!(segments.len() >= TRUNK_SEGMENTS);
            assert!(segments.len() <= TRUNK_SEGMENTS + MAX_BRANCHES * BRANCH_SEGMENTS);
            for segment in &segments {
                assert!(
                    segment
                        .start
                        .iter()
                        .chain(&segment.end)
                        .all(|v| v.is_finite())
                );
            }
            assert!(
                segments[TRUNK_SEGMENTS..]
                    .iter()
                    .all(|segment| segment.half_width < segments[0].half_width)
            );
        }
    }
}
