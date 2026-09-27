use assets::{EntityAnimationKeyframe, EntityAnimationProperty};

use super::{tick::WeightedClip, *};

#[derive(Clone, Copy)]
pub(super) struct LocalDelta {
    pub(super) translation: [f32; 3],
    pub(super) rotation: [f32; 3],
    pub(super) scale: [f32; 3],
}

impl Default for LocalDelta {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

impl LocalDelta {
    fn property(&mut self, property: EntityAnimationProperty) -> &mut [f32; 3] {
        match property {
            EntityAnimationProperty::Translation => &mut self.translation,
            EntityAnimationProperty::Rotation => &mut self.rotation,
            EntityAnimationProperty::Scale => &mut self.scale,
        }
    }
}

/// Blends weighted clips into per-bone deltas, evaluating keyframe expressions against the
/// value earlier clips produced for the same channel.
pub(super) fn sample_clips(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    bone_count: usize,
    clips: &[WeightedClip],
    budget: &mut EvalBudget<'_>,
) -> Result<Vec<LocalDelta>, EvalError> {
    let assets = evaluator.assets;
    let mut local = vec![LocalDelta::default(); bone_count];
    for weighted in clips {
        budget.charge_work()?;
        let weight = weighted.weight;
        let clip = assets
            .animation_clips()
            .get(weighted.clip)
            .ok_or(EvalError::Invalid)?;
        let length = clip.length_seconds.get();
        // A clip's own clock starts when its controller state was entered.
        let clip_tick = evaluator.anim_tick.saturating_sub(weighted.started_tick);
        let evaluator = &Evaluator {
            anim_tick: clip_tick,
            ..*evaluator
        };
        let raw_time = clip_tick as f32 * 0.05;
        let time = match clip.loop_mode {
            EntityAnimationLoop::Loop if length > 0.0 => raw_time.rem_euclid(length),
            EntityAnimationLoop::Once | EntityAnimationLoop::HoldOnLastFrame => {
                raw_time.clamp(0.0, length)
            }
            EntityAnimationLoop::Loop => 0.0,
        };
        let first = clip.first_channel as usize;
        let end = first
            .checked_add(clip.channel_count as usize)
            .ok_or(EvalError::Invalid)?;
        for channel in assets
            .animation_channels()
            .get(first..end)
            .ok_or(EvalError::Invalid)?
        {
            budget.charge_work()?;
            let bone = local
                .get_mut(channel.bone as usize)
                .ok_or(EvalError::Invalid)?;
            let identity = LocalDelta::default();
            if clip.override_previous {
                let mut reset = identity;
                *bone.property(channel.property) = *reset.property(channel.property);
            }
            let current = bone.property(channel.property);
            let this = *current;
            let value = sample_channel(
                assets,
                channel.first_keyframe,
                channel.keyframe_count,
                time,
                |keyframe| keyframe_value(evaluator, variables, keyframe, this, budget),
            )?;
            for (axis, value) in value.into_iter().enumerate() {
                if channel.property == EntityAnimationProperty::Scale {
                    current[axis] *= 1.0 + (value - 1.0) * weight;
                } else {
                    current[axis] += value * weight;
                }
            }
        }
    }
    Ok(local)
}

fn keyframe_value(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    keyframe: &EntityAnimationKeyframe,
    this: [f32; 3],
    budget: &mut EvalBudget<'_>,
) -> Result<[f32; 3], EvalError> {
    let mut value = keyframe.value.map(|value| value.get());
    for axis in 0..3 {
        if let Some(expression) = keyframe.expressions[axis] {
            value[axis] = evaluator.number(expression as usize, variables, this[axis], budget)?;
        }
    }
    Ok(value)
}

fn sample_channel(
    assets: &RuntimeEntityAssets,
    first: u32,
    count: u32,
    time: f32,
    mut value: impl FnMut(&EntityAnimationKeyframe) -> Result<[f32; 3], EvalError>,
) -> Result<[f32; 3], EvalError> {
    let first = first as usize;
    let frames = assets
        .animation_keyframes()
        .get(
            first
                ..first
                    .checked_add(count as usize)
                    .ok_or(EvalError::Invalid)?,
        )
        .ok_or(EvalError::Invalid)?;
    let first_frame = frames.first().ok_or(EvalError::Invalid)?;
    if time < first_frame.time_seconds.get() {
        return value(first_frame);
    }
    let exact_end = frames.partition_point(|frame| frame.time_seconds.get() <= time);
    if exact_end > 0 && frames[exact_end - 1].time_seconds.get() == time {
        return value(&frames[exact_end - 1]);
    }
    if exact_end == frames.len() {
        return value(&frames[frames.len() - 1]);
    }
    let left_index = exact_end - 1;
    let right_index = exact_end;
    let left = &frames[left_index];
    let right = &frames[right_index];
    let left_time = left.time_seconds.get();
    let right_time = right.time_seconds.get();
    let amount = ((time - left_time) / (right_time - left_time)).clamp(0.0, 1.0);
    let left_value = value(left)?;
    match left.interpolation {
        EntityAnimationInterpolation::Step => Ok(left_value),
        EntityAnimationInterpolation::Linear => Ok(lerp3(left_value, value(right)?, amount)),
        EntityAnimationInterpolation::CatmullRom => {
            let right_value = value(right)?;
            let previous = match frames.get(left_index.wrapping_sub(1)) {
                Some(frame) if left_index > 0 => value(frame)?,
                _ => left_value,
            };
            let next = match frames.get(right_index + 1) {
                Some(frame) => value(frame)?,
                None => right_value,
            };
            Ok(std::array::from_fn(|axis| {
                catmull(
                    previous[axis],
                    left_value[axis],
                    right_value[axis],
                    next[axis],
                    amount,
                )
            }))
        }
    }
}

pub(super) fn compose_pose(
    bones: &[RuntimeBone],
    local: &[LocalDelta],
) -> Option<Vec<BoneTransform>> {
    let mut transforms = vec![None; bones.len()];
    let mut visiting = vec![false; bones.len()];
    for index in 0..bones.len() {
        compose_bone(index, bones, local, &mut transforms, &mut visiting)?;
    }
    transforms.into_iter().collect()
}

fn compose_bone(
    index: usize,
    bones: &[RuntimeBone],
    local: &[LocalDelta],
    transforms: &mut [Option<BoneTransform>],
    visiting: &mut [bool],
) -> Option<BoneTransform> {
    if let Some(transform) = transforms.get(index).copied().flatten() {
        return Some(transform);
    }
    if *visiting.get(index)? {
        return None;
    }
    visiting[index] = true;
    let bone = bones.get(index)?;
    let delta = local.get(index).copied().unwrap_or_default();
    if (delta.scale[0] - delta.scale[1]).abs() > f32::EPSILON
        || (delta.scale[0] - delta.scale[2]).abs() > f32::EPSILON
    {
        return None;
    }
    // Pivots are already in the X-mirrored rig frame; authored offsets and angles are not.
    let translation = std::array::from_fn(|axis| {
        let parent_pivot = bone
            .parent
            .and_then(|parent| bones.get(parent))
            .map_or(0.0, |parent| parent.pivot[axis]);
        let offset = if axis == 0 {
            -delta.translation[axis]
        } else {
            delta.translation[axis]
        };
        bone.pivot[axis] - parent_pivot + offset
    });
    let [x, y, z] = std::array::from_fn(|axis| bone.rotation[axis] + delta.rotation[axis]);
    // Authored X and Y angles turn against the right-hand rule in the mirrored frame.
    let rotation = quat_from_euler([-x, -y, z]);
    let scale = delta.scale[0];
    let transform = if let Some(parent_index) = bone.parent {
        let parent = compose_bone(parent_index, bones, local, transforms, visiting)?;
        let parent_scale = parent.translation_scale[3];
        let scaled = translation.map(|value| value * parent_scale);
        let rotated = rotate_vector(parent.rotation, scaled);
        BoneTransform {
            rotation: quat_multiply(parent.rotation, rotation),
            translation_scale: [
                parent.translation_scale[0] + rotated[0],
                parent.translation_scale[1] + rotated[1],
                parent.translation_scale[2] + rotated[2],
                parent_scale * scale,
            ],
        }
    } else {
        BoneTransform {
            rotation,
            translation_scale: [translation[0], translation[1], translation[2], scale],
        }
    };
    if transform
        .rotation
        .iter()
        .chain(transform.translation_scale.iter())
        .any(|value| !value.is_finite())
    {
        return None;
    }
    visiting[index] = false;
    transforms[index] = Some(transform);
    Some(transform)
}

pub(super) fn quat_from_euler(rotation: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = rotation.map(|value| value.to_radians() * 0.5);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ]
}

fn quat_multiply(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

fn rotate_vector(rotation: [f32; 4], vector: [f32; 3]) -> [f32; 3] {
    let qvector = [vector[0], vector[1], vector[2], 0.0];
    let inverse = [-rotation[0], -rotation[1], -rotation[2], rotation[3]];
    let result = quat_multiply(quat_multiply(rotation, qvector), inverse);
    [result[0], result[1], result[2]]
}

fn lerp3(left: [f32; 3], right: [f32; 3], amount: f32) -> [f32; 3] {
    std::array::from_fn(|axis| left[axis] + (right[axis] - left[axis]) * amount)
}

fn catmull(p0: f32, p1: f32, p2: f32, p3: f32, amount: f32) -> f32 {
    let amount2 = amount * amount;
    let amount3 = amount2 * amount;
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * amount
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * amount2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * amount3)
}
