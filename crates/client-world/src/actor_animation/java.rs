//! Java Edition 1.7 per-tick player motion and the render-time retargeting its pose uses.
//! Rules: docs/reference/java-1-7-animations.md.

use std::sync::Arc;

use super::{
    BoneTransform, RuntimeBone,
    pose::{quat_multiply, rotate_vector, total_scale, with_scale},
    query::wrap_degrees,
};

/// Previous and current tick values of Java's player motion.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct JavaMotion {
    pub limb_swing: [f32; 2],
    pub limb_amount: [f32; 2],
    /// Body yaw in degrees.
    pub body_yaw: [f32; 2],
    /// First-person equip progress.
    pub equip: [f32; 2],
    pub riding: bool,
    /// Swimming, crawling, gliding, sleeping or emoting: postures Java has no pose for.
    pub vanilla_posture: bool,
    /// The cape's chasing point less the position, in blocks.
    pub cape: [[f32; 3]; 2],
    /// Walk bob amplitude, eased toward the capped ground speed.
    pub bob: [f32; 2],
    /// Walked distance scaled 0.6 per block; only the local player walks it.
    pub walked: [f32; 2],
}

#[derive(Clone, Debug)]
pub(super) struct JavaMotionState {
    pub(super) motion: JavaMotion,
    equipped: Option<Arc<str>>,
    hurt_time: u8,
    reset_equip: bool,
    chase: Option<[f64; 3]>,
}

/// One tick of what Java's motion reads.
pub(super) struct JavaTick<'a> {
    pub(super) delta: [f32; 3],
    pub(super) yaw: f32,
    pub(super) swinging: bool,
    pub(super) hurt_time: u8,
    pub(super) held: &'a Option<Arc<str>>,
    pub(super) riding: bool,
    pub(super) vanilla_posture: bool,
    pub(super) position: [f32; 3],
    /// Blocks per tick.
    pub(super) velocity: [f32; 3],
    pub(super) on_ground: bool,
    pub(super) sneaking: bool,
    /// The client's own player, the only one Java advances a walk distance for.
    pub(super) local: bool,
}

const BODY_FOLLOW: f32 = 0.3;
const HEAD_LIMIT: f32 = 75.0;
const HEAD_SOFT_LIMIT_SQUARED: f32 = 2500.0;
const HEAD_SOFT_PULL: f32 = 0.2;
const FACING_DISTANCE_SQUARED: f32 = 0.002_500_000_2;
const LIMB_GAIN: f64 = 4.0;
const LIMB_FOLLOW: f32 = 0.4;
const HURT_LIMB_AMOUNT: f32 = 1.5;
const EQUIP_STEP: f32 = 0.4;
const EQUIP_SWAP: f32 = 0.1;
const CAPE_FOLLOW: f64 = 0.25;
const CAPE_SNAP_BLOCKS: f64 = 10.0;
const BOB_CAP: f32 = 0.1;
const BOB_FOLLOW: f32 = 0.4;
const WALK_PER_BLOCK: f32 = 0.6;

impl JavaMotionState {
    pub(super) fn spawn(body_yaw: f32) -> Self {
        Self {
            motion: JavaMotion {
                limb_swing: [0.0; 2],
                limb_amount: [0.0; 2],
                body_yaw: [body_yaw; 2],
                equip: [1.0; 2],
                riding: false,
                vanilla_posture: false,
                cape: [[0.0; 3]; 2],
                bob: [0.0; 2],
                walked: [0.0; 2],
            },
            equipped: None,
            hurt_time: 0,
            reset_equip: false,
            chase: None,
        }
    }

    pub(super) fn advance(&mut self, tick: &JavaTick<'_>) {
        let motion = &mut self.motion;
        motion.riding = tick.riding;
        motion.vanilla_posture = tick.vanilla_posture;
        let [dx, _, dz] = tick.delta;
        if tick.hurt_time > self.hurt_time {
            motion.limb_amount[1] = HURT_LIMB_AMOUNT;
        }
        self.hurt_time = tick.hurt_time;
        motion.limb_amount[0] = motion.limb_amount[1];
        let target = ((f64::from(dx).powi(2) + f64::from(dz).powi(2)).sqrt() * LIMB_GAIN) as f32;
        motion.limb_amount[1] += (target.min(1.0) - motion.limb_amount[1]) * LIMB_FOLLOW;
        motion.limb_swing[0] = motion.limb_swing[1];
        motion.limb_swing[1] += motion.limb_amount[1];

        motion.body_yaw[0] = motion.body_yaw[1];
        let mut body = motion.body_yaw[1];
        let mut facing = body;
        if dx * dx + dz * dz > FACING_DISTANCE_SQUARED {
            facing = (f64::from(dz).atan2(f64::from(dx)) as f32).to_degrees() - 90.0;
        }
        if tick.swinging {
            facing = tick.yaw;
        }
        body += wrap_degrees(facing - body) * BODY_FOLLOW;
        let lag = wrap_degrees(tick.yaw - body).clamp(-HEAD_LIMIT, HEAD_LIMIT);
        body = tick.yaw - lag;
        if lag * lag > HEAD_SOFT_LIMIT_SQUARED {
            body += lag * HEAD_SOFT_PULL;
        }
        motion.body_yaw[1] = body;

        // A placement lowers the item before this tick's rise.
        if std::mem::take(&mut self.reset_equip) {
            motion.equip[1] = 0.0;
        }
        motion.equip[0] = motion.equip[1];
        let target = if self.equipped == *tick.held {
            1.0
        } else {
            0.0
        };
        motion.equip[1] += (target - motion.equip[1]).clamp(-EQUIP_STEP, EQUIP_STEP);
        if motion.equip[1] < EQUIP_SWAP {
            self.equipped.clone_from(tick.held);
        }
        self.advance_cape(tick);
    }

    /// The cape's chasing point, walk bob and walk distance, after the movement this tick.
    fn advance_cape(&mut self, tick: &JavaTick<'_>) {
        let motion = &mut self.motion;
        let position = tick.position.map(f64::from);
        let previous_position: [f64; 3] =
            std::array::from_fn(|axis| position[axis] - f64::from(tick.delta[axis]));
        let mut chase = self.chase.unwrap_or(position);
        let mut previous = chase;
        for axis in 0..3 {
            let lag = position[axis] - chase[axis];
            if lag.abs() > CAPE_SNAP_BLOCKS {
                previous[axis] = position[axis];
                chase[axis] = position[axis];
            }
            // Java adds the pre-snap lag even after snapping.
            chase[axis] += lag * CAPE_FOLLOW;
        }
        self.chase = Some(chase);
        motion.cape = [
            std::array::from_fn(|axis| (previous[axis] - previous_position[axis]) as f32),
            std::array::from_fn(|axis| (chase[axis] - position[axis]) as f32),
        ];
        let [vx, _, vz] = tick.velocity;
        let speed = if tick.on_ground {
            vx.hypot(vz).min(BOB_CAP)
        } else {
            0.0
        };
        motion.bob = [
            motion.bob[1],
            motion.bob[1] + (speed - motion.bob[1]) * BOB_FOLLOW,
        ];
        let walks = tick.local && !tick.riding && !(tick.on_ground && tick.sneaking);
        let [dx, _, dz] = tick.delta;
        let step = if walks {
            dx.hypot(dz) * WALK_PER_BLOCK
        } else {
            0.0
        };
        motion.walked = [motion.walked[1], motion.walked[1] + step];
    }

    pub(super) fn reset_equip(&mut self) {
        self.reset_equip = true;
    }

    /// The item Java's first-person hand still draws while the equip dips.
    pub(super) fn equipped(&self) -> Option<&Arc<str>> {
        self.equipped.as_ref()
    }
}

/// The pose at `alpha` with `targets` (model-space bones by index) replacing their joints;
/// every other bone keeps its animated transform relative to its parent.
pub(super) fn retarget(
    bones: &[RuntimeBone],
    previous: &[BoneTransform],
    current: &[BoneTransform],
    alpha: f32,
    targets: &[Option<BoneTransform>],
) -> Option<Vec<BoneTransform>> {
    if previous.len() != bones.len() || current.len() != bones.len() {
        return None;
    }
    let mut posed: Vec<Option<BoneTransform>> = vec![None; bones.len()];
    for index in 0..bones.len() {
        retarget_bone(
            index, bones, previous, current, alpha, targets, &mut posed, 0,
        )?;
    }
    posed.into_iter().collect()
}

#[allow(clippy::too_many_arguments)]
fn retarget_bone(
    index: usize,
    bones: &[RuntimeBone],
    previous: &[BoneTransform],
    current: &[BoneTransform],
    alpha: f32,
    targets: &[Option<BoneTransform>],
    posed: &mut [Option<BoneTransform>],
    depth: usize,
) -> Option<BoneTransform> {
    if let Some(done) = posed[index] {
        return Some(done);
    }
    if depth > bones.len() {
        return None;
    }
    let bone = match (targets.get(index).copied().flatten(), bones[index].parent) {
        (Some(target), _) => target,
        (None, None) => blend(previous[index], current[index], alpha),
        (None, Some(parent)) => {
            let local = blend(
                relative(previous[parent], previous[index]),
                relative(current[parent], current[index]),
                alpha,
            );
            let parent = retarget_bone(
                parent,
                bones,
                previous,
                current,
                alpha,
                targets,
                posed,
                depth + 1,
            )?;
            compose(parent, local)
        }
    };
    posed[index] = Some(bone);
    Some(bone)
}

fn translation(bone: BoneTransform) -> [f32; 3] {
    [
        bone.translation_scale[0],
        bone.translation_scale[1],
        bone.translation_scale[2],
    ]
}

/// `child` in `parent`'s frame, matching the pose composer's scale handling.
fn relative(parent: BoneTransform, child: BoneTransform) -> BoneTransform {
    let inverse = conjugate(parent.rotation);
    // A parent hidden by a zero scale leaves its children's offsets unscaled.
    let parent_scale = total_scale(&parent).map(|scale| {
        if scale.abs() > f32::EPSILON {
            scale
        } else {
            1.0
        }
    });
    let offset: [f32; 3] =
        std::array::from_fn(|axis| translation(child)[axis] - translation(parent)[axis]);
    let local = rotate_vector(inverse, offset);
    let child_scale = total_scale(&child);
    with_scale(
        quat_multiply(inverse, child.rotation),
        std::array::from_fn(|axis| local[axis] / parent_scale[axis]),
        std::array::from_fn(|axis| child_scale[axis] / parent_scale[axis]),
    )
}

fn compose(parent: BoneTransform, local: BoneTransform) -> BoneTransform {
    let parent_scale = total_scale(&parent);
    let scaled = std::array::from_fn(|axis| translation(local)[axis] * parent_scale[axis]);
    let offset = rotate_vector(parent.rotation, scaled);
    let local_scale = total_scale(&local);
    with_scale(
        quat_multiply(parent.rotation, local.rotation),
        std::array::from_fn(|axis| translation(parent)[axis] + offset[axis]),
        std::array::from_fn(|axis| parent_scale[axis] * local_scale[axis]),
    )
}

fn conjugate([x, y, z, w]: [f32; 4]) -> [f32; 4] {
    [-x, -y, -z, w]
}

fn blend(from: BoneTransform, to: BoneTransform, alpha: f32) -> BoneTransform {
    let lerp = |a: f32, b: f32| a + (b - a) * alpha;
    let mut end = to.rotation;
    let dot: f32 = (0..4).map(|i| from.rotation[i] * end[i]).sum();
    if dot < 0.0 {
        end = end.map(|value| -value);
    }
    let mixed: [f32; 4] = std::array::from_fn(|i| lerp(from.rotation[i], end[i]));
    let length = mixed.iter().map(|value| value * value).sum::<f32>().sqrt();
    let rotation = if length > f32::EPSILON {
        mixed.map(|value| value / length)
    } else {
        to.rotation
    };
    let (from_scale, to_scale) = (total_scale(&from), total_scale(&to));
    with_scale(
        rotation,
        std::array::from_fn(|axis| lerp(translation(from)[axis], translation(to)[axis])),
        std::array::from_fn(|axis| lerp(from_scale[axis], to_scale[axis])),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(delta: [f32; 3], yaw: f32) -> JavaTick<'static> {
        JavaTick {
            delta,
            yaw,
            swinging: false,
            hurt_time: 0,
            held: &None,
            riding: false,
            vanilla_posture: false,
            position: [0.0; 3],
            velocity: [0.0; 3],
            on_ground: true,
            sneaking: false,
            local: true,
        }
    }

    /// Limb amount eases 40% toward four times the step, capped at 1, and accumulates.
    #[test]
    fn limb_swing_follows_java_step() {
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&tick([0.2, 0.0, 0.0], 0.0));
        assert!((state.motion.limb_amount[1] - 0.32).abs() < 1e-6);
        assert!((state.motion.limb_swing[1] - 0.32).abs() < 1e-6);
        state.advance(&tick([0.05, 0.0, 0.0], 0.0));
        assert!((state.motion.limb_amount[1] - (0.32 + (0.2 - 0.32) * 0.4)).abs() < 1e-6);
        assert!((state.motion.limb_amount[0] - 0.32).abs() < 1e-6);
    }

    /// A hurt sets the amount to 1.5 before the tick eases it.
    #[test]
    fn hurt_flails_the_limbs() {
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&JavaTick {
            hurt_time: 10,
            ..tick([0.0; 3], 0.0)
        });
        assert_eq!(state.motion.limb_amount[0], 1.5);
        assert!((state.motion.limb_amount[1] - 0.9).abs() < 1e-6);
        state.advance(&JavaTick {
            hurt_time: 9,
            ..tick([0.0; 3], 0.0)
        });
        assert!((state.motion.limb_amount[1] - 0.54).abs() < 1e-6);
    }

    /// Past 50 degrees of head turn the body is pulled a fifth of the way back.
    #[test]
    fn body_yaw_lags_the_head_with_the_soft_pull() {
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&tick([0.0; 3], 70.0));
        // 0.3 follow toward its own yaw is a no-op; 70 degrees of lag pulls back 14.
        assert!((state.motion.body_yaw[1] - 14.0).abs() < 1e-4);
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&tick([0.0; 3], 120.0));
        assert!((state.motion.body_yaw[1] - (45.0 + 15.0)).abs() < 1e-4);
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&tick([0.0, 0.0, 0.2], 0.0));
        assert!(
            (state.motion.body_yaw[1] - 0.0).abs() < 1e-4,
            "moving +z faces 0"
        );
        let mut state = JavaMotionState::spawn(0.0);
        state.advance(&JavaTick {
            swinging: true,
            ..tick([0.0; 3], 40.0)
        });
        assert!((state.motion.body_yaw[1] - 12.0).abs() < 1e-4);
    }

    /// The equip dips 0.4 a tick, adopts the new item below 0.1, and a placement drops to 0.
    #[test]
    fn equip_dips_swaps_and_restarts_on_use() {
        let sword: Option<Arc<str>> = Some(Arc::from("minecraft:iron_sword"));
        let mut state = JavaMotionState::spawn(0.0);
        let heights = (0..6)
            .map(|_| {
                state.advance(&JavaTick {
                    held: &sword,
                    ..tick([0.0; 3], 0.0)
                });
                state.motion.equip[1]
            })
            .collect::<Vec<_>>();
        let expected = [0.6, 0.2, 0.0, 0.4, 0.8, 1.0];
        for (height, expected) in heights.iter().zip(expected) {
            assert!((height - expected).abs() < 1e-6, "{heights:?}");
        }
        state.reset_equip();
        state.advance(&JavaTick {
            held: &sword,
            ..tick([0.0; 3], 0.0)
        });
        assert_eq!(state.motion.equip, [0.0, 0.4]);
    }

    /// Steps a history of positions and velocities, as each tick sees them.
    fn cape_history(steps: &[([f32; 3], [f32; 3], bool)]) -> Vec<JavaMotion> {
        let mut state = JavaMotionState::spawn(0.0);
        let mut last = steps[0].0;
        steps
            .iter()
            .map(|&(position, velocity, on_ground)| {
                state.advance(&JavaTick {
                    delta: std::array::from_fn(|axis| position[axis] - last[axis]),
                    position,
                    velocity,
                    on_ground,
                    ..tick([0.0; 3], 0.0)
                });
                last = position;
                state.motion
            })
            .collect()
    }

    /// The chasing point closes a quarter of its lag a tick, trailing three steps behind a
    /// steady walk, and the bob eases toward the capped ground speed.
    #[test]
    fn cape_chase_trails_walking_and_settles_after_a_stop() {
        let mut steps = vec![([0.0, 64.0, 0.0], [0.0; 3], true)];
        for tick in 1..=40 {
            steps.push(([0.0, 64.0, tick as f32 * 0.2], [0.0, 0.0, 0.2], true));
        }
        for _ in 0..6 {
            steps.push(([0.0, 64.0, 8.0], [0.0; 3], true));
        }
        let history = cape_history(&steps);
        let mut chase = 0.0_f64;
        let mut bob = 0.0_f32;
        for (index, motion) in history.iter().enumerate() {
            let (z, speed) = (f64::from(steps[index].0[2]), steps[index].1[2]);
            chase += (z - chase) * 0.25;
            bob += (speed.min(0.1) - bob) * 0.4;
            assert!(
                (f64::from(motion.cape[1][2]) - (chase - z)).abs() < 1e-4,
                "tick {index}"
            );
            assert!((motion.bob[1] - bob).abs() < 1e-6);
        }
        assert!(
            (history[40].cape[1][2] + 0.6).abs() < 1e-3,
            "three steps behind"
        );
        assert!(history[46].cape[1][2].abs() < history[41].cape[1][2].abs());
        assert_eq!(history[0].cape, [[0.0; 3]; 2]);
    }

    /// Falling leaves the chasing point above; airborne the bob decays.
    #[test]
    fn cape_chase_lags_a_fall_and_snaps_past_ten_blocks() {
        let history = cape_history(&[
            ([0.0, 64.0, 0.0], [0.0; 3], true),
            ([0.0, 63.5, 0.0], [0.0, -0.5, 0.0], false),
            ([0.0, 62.5, 0.0], [0.0, -1.0, 0.0], false),
            ([30.0, 62.5, 0.0], [0.0; 3], false),
        ]);
        assert!((history[1].cape[1][1] - 0.375).abs() < 1e-5);
        assert!(history[2].cape[1][1] > history[1].cape[1][1]);
        // A snap re-anchors at the position, then still adds the quarter of the old lag.
        assert!((history[3].cape[1][0] - 7.5).abs() < 1e-4);
        // The previous chase also jumps to the new position while the previous position does not.
        assert_eq!(history[3].cape[0][0], 30.0);
        assert_eq!(history[2].bob[1], 0.0);
    }

    fn root(rotation: [f32; 4], translation: [f32; 3]) -> BoneTransform {
        with_scale(rotation, translation, [1.0; 3])
    }

    /// A child of a zero-scaled parent still retargets to finite transforms.
    #[test]
    fn retarget_survives_a_zero_scaled_parent() {
        let bones = vec![
            RuntimeBone::default(),
            RuntimeBone {
                parent: Some(0),
                ..Default::default()
            },
        ];
        let hidden = with_scale([0.0, 0.0, 0.0, 1.0], [0.0, 24.0, 0.0], [0.0; 3]);
        let pose = vec![hidden, root([0.0, 0.0, 0.0, 1.0], [0.0, 24.0, 3.0])];
        let target = root([0.0, 0.0, 0.0, 1.0], [1.0, 20.0, 0.0]);
        let posed = retarget(&bones, &pose, &pose, 0.0, &[Some(target), None]).unwrap();
        assert_eq!(translation(posed[1]), [1.0, 20.0, 3.0]);
    }

    /// Untargeted children keep their animated offset from the parent under its new transform.
    #[test]
    fn retarget_carries_children_with_their_animated_offsets() {
        let bones = vec![
            RuntimeBone::default(),
            RuntimeBone {
                parent: Some(0),
                ..Default::default()
            },
        ];
        let half_turn = [0.0, 0.0, 1.0, 0.0];
        let pose = vec![
            root([0.0, 0.0, 0.0, 1.0], [0.0, 24.0, 0.0]),
            root([0.0, 0.0, 0.0, 1.0], [0.0, 24.0, 3.0]),
        ];
        let target = root(half_turn, [1.0, 20.0, 0.0]);
        let posed = retarget(&bones, &pose, &pose, 0.5, &[Some(target), None]).unwrap();
        assert_eq!(posed[0], target);
        assert_eq!(translation(posed[1]), [1.0, 20.0, 3.0]);
        assert_eq!(posed[1].rotation, half_turn);
    }
}
