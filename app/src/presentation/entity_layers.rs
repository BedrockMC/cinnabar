//! Render-controller texture layers of entity bodies: the first layer replaces the body's
//! default texture, later layers draw the same rig again over it.
use std::sync::Arc;

use client_world::{ActorRigSnapshot, RenderTextureLayer};
use render::{
    ACTOR_LAYER_BODY, ActorArtworkLocation, ActorArtworkPages, ActorRigSubmission,
    RenderBoneTransform, pack_overlay_rgba8,
};

use super::actors::ActorPresentationBatch;

/// First render layer id of extra texture layers; equipment layers use the ids below.
pub(crate) const ACTOR_LAYER_TEXTURE_BASE: u8 = 32;
const MAX_TEXTURE_LAYERS: usize = 8;

struct ResolvedLayer {
    location: ActorArtworkLocation,
    tint: u32,
    overlay: Option<u32>,
    hidden_bones: Arc<[u32]>,
    uv_anim: [f32; 4],
}

/// Packs a colour multiplier as the instance tint word; white leaves the texture untouched.
fn pack_layer_tint(color: [f32; 4]) -> u32 {
    let byte = |value: f32| {
        if value.is_finite() {
            (value.clamp(0.0, 1.0) * 255.0).round() as u32
        } else {
            255
        }
    };
    let [red, green, blue] = [color[0], color[1], color[2]].map(byte);
    if (red, green, blue) == (255, 255, 255) {
        0
    } else {
        0xff00_0000 | (blue << 16) | (green << 8) | red
    }
}

fn resolve(
    submission: &ActorRigSubmission,
    layers: &[RenderTextureLayer],
    artwork: &ActorArtworkPages,
) -> Vec<ResolvedLayer> {
    layers
        .iter()
        .filter_map(|layer| {
            Some(ResolvedLayer {
                location: artwork.variant_location(submission.input.rig, layer.source)?,
                tint: pack_layer_tint(layer.color),
                overlay: (layer.overlay[3] > 0.0).then(|| pack_overlay_rgba8(layer.overlay)),
                hidden_bones: Arc::clone(&layer.hidden_bones),
                uv_anim: layer.uv_anim,
            })
        })
        .take(MAX_TEXTURE_LAYERS)
        .collect()
}

/// The zero-scale pose vanilla uses to hide a bone.
fn hide_bones(poses: &Arc<[RenderBoneTransform]>, hidden: &[u32]) -> Arc<[RenderBoneTransform]> {
    let mut poses = poses.to_vec();
    for &index in hidden {
        if let Some(bone) = poses.get_mut(index as usize) {
            bone.translation_scale = [0.0; 4];
        }
    }
    poses.into()
}

fn layered(body: &ActorRigSubmission, layer: &ResolvedLayer, index: usize) -> ActorRigSubmission {
    let mut submission = body.clone();
    if index > 0 {
        submission.input.identity.layer = ACTOR_LAYER_TEXTURE_BASE + (index - 1) as u8;
    }
    submission.texture_layer = layer.location.layer();
    submission.tint = layer.tint;
    submission.uv_anim = layer.uv_anim;
    if let Some(overlay) = layer.overlay {
        submission.overlay_rgba8 = overlay;
    }
    if !layer.hidden_bones.is_empty() {
        submission.input.previous_bones =
            hide_bones(&submission.input.previous_bones, &layer.hidden_bones);
        submission.input.current_bones =
            hide_bones(&submission.input.current_bones, &layer.hidden_bones);
    }
    submission
}

/// Applies each entity body's selected texture layers to the batch: the first replaces the
/// body's texture, the rest are appended as extra layers of the same actor.
pub(crate) fn apply_render_layers<'a>(
    batch: &mut ActorPresentationBatch,
    rig_of: impl Fn(u64) -> Option<ActorRigSnapshot<'a>>,
    artwork: &ActorArtworkPages,
) {
    let mut extras = Vec::new();
    for index in 0..batch.submissions.len() {
        let body = &batch.submissions[index];
        let identity = body.input.identity;
        if identity.layer != ACTOR_LAYER_BODY || !batch.artwork.contains_key(&identity) {
            continue;
        }
        let Some(rig) = rig_of(identity.runtime_id) else {
            continue;
        };
        let layers = resolve(body, rig.render, artwork);
        if layers.is_empty() {
            continue;
        }
        let pristine = body.clone();
        for (layer_index, layer) in layers.iter().enumerate() {
            let submission = layered(&pristine, layer, layer_index);
            batch
                .artwork
                .insert(submission.input.identity, layer.location);
            if layer_index == 0 {
                batch.submissions[index] = submission;
            } else {
                extras.push(submission);
            }
        }
    }
    batch.submissions.extend(extras);
}

#[cfg(test)]
mod tests {
    use super::pack_layer_tint;

    #[test]
    fn white_is_untinted_and_other_colours_enable_the_tint_word() {
        assert_eq!(pack_layer_tint([1.0, 1.0, 1.0, 1.0]), 0);
        assert_eq!(pack_layer_tint([1.0, 0.0, 0.0, 1.0]), 0xff00_00ff);
        assert_eq!(pack_layer_tint([0.0, 0.0, 1.0, 1.0]), 0xffff_0000);
    }
}
