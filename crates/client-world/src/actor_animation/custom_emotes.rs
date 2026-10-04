//! The owned emote channels reuse native named skeleton and hierarchy composition.
use super::{
    RuntimeBone, geometry,
    pose::{LocalDelta, compose_pose},
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
    let previous_time = previous;
    let previous: Arc<[BoneTransform]> =
        compose_pose(bones, &channels(bones, names, emote, previous, leg_height))?.into();
    let current = if previous_time == current {
        Arc::clone(&previous)
    } else {
        compose_pose(bones, &channels(bones, names, emote, current, leg_height))?.into()
    };
    Some((previous, current))
}

fn channels(
    bones: &[RuntimeBone],
    names: &[Box<str>],
    emote: CustomEmote,
    seconds: f64,
    leg_height: f32,
) -> Vec<LocalDelta> {
    match emote {
        CustomEmote::Twerk => twerk_channels(bones, names, emote, seconds, leg_height),
    }
}

fn twerk_channels(
    bones: &[RuntimeBone],
    names: &[Box<str>],
    emote: CustomEmote,
    seconds: f64,
    leg_height: f32,
) -> Vec<LocalDelta> {
    let angle = (seconds.rem_euclid(emote.duration_seconds()) / emote.duration_seconds()
        * std::f64::consts::TAU) as f32;
    let wave = angle.sin();
    let leg_pitch = 22.0 + 6.0 * wave;
    let spread = 20.0_f32;
    let dip = leg_height * (1.0 - leg_pitch.to_radians().cos() * spread.to_radians().cos());
    let waist = names.iter().any(|name| name.as_ref() == "waist");
    bones
        .iter()
        .zip(names)
        .map(|(bone, name)| {
            let mut local = LocalDelta::default();
            if bone.parent.is_none() {
                local.translation[1] = -dip;
            }
            match name.as_ref() {
                "waist" => local.rotation = [30.0 + 8.0 * wave, 4.0 * angle.cos(), 2.0 * wave],
                "body" if !waist => {
                    local.rotation = [30.0 + 8.0 * wave, 4.0 * angle.cos(), 2.0 * wave]
                }
                "head" => local.rotation = [-14.0, 0.0, 0.0],
                "leftarm" => local.rotation = [-28.0 - 5.0 * wave, 0.0, -8.0],
                "rightarm" => local.rotation = [-28.0 - 5.0 * wave, 0.0, 8.0],
                "leftleg" => local.rotation = [-leg_pitch, 0.0, -spread],
                "rightleg" => local.rotation = [-leg_pitch, 0.0, spread],
                _ => {}
            }
            local
        })
        .collect()
}

#[cfg(test)]
mod tests;
