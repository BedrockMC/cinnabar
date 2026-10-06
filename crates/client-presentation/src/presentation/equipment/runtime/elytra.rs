//! Worn wings use the pack's controller, movement queries and body attachment.

use client_world::AttachableAnimationInput;

use super::*;

impl EquipmentRuntime {
    /// Samples the authored worn controller and places its model beneath the owner's body bone.
    pub(super) fn push_elytra(
        &mut self,
        body: &ActorRigSubmission,
        item: &WornItem,
        equipment: &ActorEquipmentInput,
        animation: EquipmentAnimation<'_>,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let Some((_, from_pack)) = self.binding_source(&item.identifier) else {
            return;
        };
        let assets = if from_pack {
            Arc::clone(&self.pack.as_ref().unwrap().assets)
        } else {
            Arc::clone(&self.assets)
        };
        let runtime = if from_pack {
            &mut self.pack.as_mut().unwrap().attachables
        } else {
            &mut self.attachables
        };
        let variables = [("variable.is_enchanted", f32::from(item.enchanted))];
        let input = equipment.attachable_input(AttachableAnimationInput {
            worn: true,
            frame_alpha: animation.frame_alpha,
            owner_variables: &variables,
            ..Default::default()
        });
        let Some(evaluated) =
            runtime.evaluate(&item.identifier, animation.owner, animation.rig, input)
        else {
            return;
        };
        let Some(selected) = evaluated.render.first() else {
            return;
        };
        let index = selected.geometry.unwrap_or(evaluated.geometry);
        let alternate = index != evaluated.geometry;
        let pose = if selected.pose.is_empty() {
            evaluated.pose
        } else {
            &selected.pose
        }
        .to_vec();
        let hidden = Arc::clone(&selected.hidden_bones);
        let model_scale = evaluated.axis_scale.map(|axis| axis * evaluated.scale);
        let material = render::ActorMaterial {
            kind: if item.enchanted {
                assets::EntityRenderMaterial::Glint
            } else {
                selected.material
            },
            state: Some(assets::EntityRenderMaterialState {
                alpha_test: true,
                cull: false,
                ..Default::default()
            }),
            glint: render::ActorGlint {
                time_seconds: (animation.rig.completed_tick as f32 + animation.frame_alpha)
                    * client_world::ACTOR_TICK_DURATION.as_secs_f32(),
                ..Default::default()
            },
            ..Default::default()
        };
        let Some(source) = assets.sources().get(selected.source as usize) else {
            return;
        };
        let texture = source
            .path
            .strip_suffix(".png")
            .or_else(|| source.path.strip_suffix(".tga"));
        let Some(location) = texture.and_then(|texture| self.texture_location(texture, from_pack))
        else {
            return;
        };
        let Some(geometry) = assets.geometries().get(index as usize) else {
            return;
        };
        let Some(geometry) = self.armor_geometry_for(&geometry.identifier, from_pack) else {
            return;
        };
        // The scene registers only each binding's default model.
        if alternate && !self.selected_geometries.contains(&geometry.rig) {
            let Some(mesh) =
                render_model::equipment_geometry(&assets, index as usize, geometry.rig)
            else {
                return;
            };
            self.pending.push(mesh);
            self.selected_geometries.insert(geometry.rig);
        }
        let Some((_, bones)) = self.body_bones_for(body.input.rig) else {
            return;
        };
        let Some(parent) = bones
            .names
            .iter()
            .position(|name| name.eq_ignore_ascii_case("body"))
        else {
            return;
        };
        let Some(mut parent) = body
            .input
            .previous_bones
            .get(parent)
            .zip(body.input.current_bones.get(parent))
            .and_then(|(previous, current)| {
                modern::interpolate_parent(*previous, *current, animation.frame_alpha)
            })
        else {
            return;
        };
        for (axis, scale) in model_scale.into_iter().enumerate() {
            parent.axis_scale[axis] *= scale;
        }
        let Some(pose) = pose
            .iter()
            .enumerate()
            .map(|(index, bone)| {
                if hidden.contains(&(index as u32)) {
                    Some(hidden_bone())
                } else {
                    modern::compose_parent(parent, *bone)
                }
            })
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        let poses = self.poses.share(body, LAYER_CHESTPLATE, [&pose, &pose]);
        let mut layer =
            layer_presentation(body, LAYER_CHESTPLATE, geometry.rig, poses, location, 0);
        layer.submission.material = material;
        layers.push(layer);
    }
}
