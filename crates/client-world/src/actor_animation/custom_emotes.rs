//! The owned emote channels reuse native named skeleton and hierarchy composition.
use super::{
    RuntimeBone, geometry,
    pose::{
        compose_pose, compose_pose_with_targets, quat_from_euler, quat_multiply, rotate_vector,
    },
};
use crate::{
    ActorRigSnapshot, BoneTransform,
    custom_emotes::{CustomEmote, CustomEmotePose},
};
use std::sync::Arc;

type PosePair = (Arc<[BoneTransform]>, Arc<[BoneTransform]>);

pub(crate) fn sample(
    rig: &ActorRigSnapshot<'_>,
    emote: CustomEmote,
    previous_seconds: f64,
    current_seconds: f64,
) -> Option<CustomEmotePose> {
    if ![previous_seconds, current_seconds]
        .into_iter()
        .all(|time| time.is_finite() && time >= 0.0)
    {
        return None;
    }
    let catalog = rig.geometry_source();
    let (bones, names) = if let Some(skin) = rig.skin_geometry {
        geometry::skeleton(&skin.bones)?
    } else {
        let (assets, geometry) = catalog?;
        geometry::resolve_bones(assets, geometry)?
    };
    if names.as_slice() != rig.bone_names
        || bones.len() != rig.current.len()
        || !["body", "head", "leftarm", "rightarm", "leftleg", "rightleg"]
            .into_iter()
            .all(|part| names.iter().any(|name| name.as_ref() == part))
    {
        return None;
    }
    let leg_height =
        bones[names.iter().position(|name| name.as_ref() == "leftleg")?].pivot[1].abs();
    let (previous, current) = pair(
        &bones,
        &names,
        emote,
        previous_seconds,
        current_seconds,
        leg_height,
    )?;
    let mut render = rig.render.to_vec();
    for layer in &mut render {
        if let Some(index) = layer.geometry {
            let (assets, _) = catalog?;
            let (bones, names) = geometry::resolve_bones(assets, index as usize)?;
            let (previous, current) = pair(
                &bones,
                &names,
                emote,
                previous_seconds,
                current_seconds,
                leg_height,
            )?;
            layer.previous_pose = previous;
            layer.pose = current;
        }
    }
    let mut skin_layers = rig.skin_layers.to_vec();
    for layer in &mut skin_layers {
        let (bones, names) = geometry::skeleton(&layer.geometry.bones)?;
        let (previous, current) = pair(
            &bones,
            &names,
            emote,
            previous_seconds,
            current_seconds,
            leg_height,
        )?;
        layer.previous = previous;
        layer.current = current;
    }
    Some(CustomEmotePose {
        previous,
        current,
        render,
        skin_layers,
    })
}

fn pair(
    bones: &[RuntimeBone],
    names: &[Box<str>],
    emote: CustomEmote,
    previous: f64,
    current: f64,
    leg_height: f32,
) -> Option<PosePair> {
    let rest = compose_pose(bones, &[])?;
    let previous_time = previous;
    let previous: Arc<[BoneTransform]> = compose_pose_with_targets(
        bones,
        &[],
        &targets(bones, names, &rest, emote, previous, leg_height),
    )?
    .into();
    let current = if previous_time == current {
        Arc::clone(&previous)
    } else {
        compose_pose_with_targets(
            bones,
            &[],
            &targets(bones, names, &rest, emote, current, leg_height),
        )?
        .into()
    };
    Some((previous, current))
}

fn targets(
    bones: &[RuntimeBone],
    names: &[Box<str>],
    rest: &[BoneTransform],
    emote: CustomEmote,
    seconds: f64,
    leg_height: f32,
) -> Vec<Option<BoneTransform>> {
    match emote {
        CustomEmote::Twerk => twerk_targets(bones, names, rest, emote, seconds, leg_height),
    }
}

fn twerk_targets(
    bones: &[RuntimeBone],
    names: &[Box<str>],
    rest: &[BoneTransform],
    emote: CustomEmote,
    seconds: f64,
    leg_height: f32,
) -> Vec<Option<BoneTransform>> {
    let angle = (seconds.rem_euclid(emote.duration_seconds()) / emote.duration_seconds()
        * std::f64::consts::TAU) as f32;
    // Owned clip from the public video: a sustained deep squat, level head and
    // hands beside the thighs, with a hip pulse instead of a standing side sway.
    let wave = angle.cos();
    let leg_pitch = 52.0 + 4.0 * wave;
    let lean = 46.0 + 7.0 * wave;
    let spread = 15.0_f32;
    let dip = leg_height * (1.0 - leg_pitch.to_radians().cos() * spread.to_radians().cos());
    // Keep the leg bottom-face centers fixed in Y/Z throughout the pulse.
    let offset = [0.0, -dip, leg_height * leg_pitch.to_radians().sin()];
    let hips = [0.0, leg_height, 0.0];
    let torso = quat_from_euler([-lean, 0.0, 0.0]);
    bones
        .iter()
        .zip(names)
        .zip(rest)
        .map(|((bone, name), rest)| {
            let (rotation, tilt_position) = match name.as_ref() {
                "waist" | "body" => (torso, true),
                "head" => ([0.0, 0.0, 0.0, 1.0], true),
                "leftarm" => (quat_from_euler([-8.0, 0.0, -3.0]), true),
                "rightarm" => (quat_from_euler([-8.0, 0.0, 3.0]), true),
                "leftleg" => (quat_from_euler([leg_pitch, 0.0, -spread]), false),
                "rightleg" => (quat_from_euler([leg_pitch, 0.0, spread]), false),
                _ if bone.parent.is_none() => ([0.0, 0.0, 0.0, 1.0], false),
                _ => return None,
            };
            let pivot: [f32; 3] = std::array::from_fn(|axis| rest.translation_scale[axis]);
            let pivot = if tilt_position {
                let relative = std::array::from_fn(|axis| pivot[axis] - hips[axis]);
                let rotated = rotate_vector(torso, relative);
                std::array::from_fn(|axis| hips[axis] + rotated[axis])
            } else {
                pivot
            };
            let mut target = *rest;
            target.rotation = quat_multiply(rotation, rest.rotation);
            for axis in 0..3 {
                target.translation_scale[axis] = pivot[axis] + offset[axis];
            }
            Some(target)
        })
        .collect()
}

#[cfg(test)]
mod tests;
