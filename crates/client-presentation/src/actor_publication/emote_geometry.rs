use crate::presentation::{equipment::EquipmentRuntime, skin_rig::SkinRigCache};

pub(super) fn apply(
    rig: &client_world::ActorRigSnapshot<'_>,
    cache: &mut SkinRigCache,
    mut equipment: Option<&mut EquipmentRuntime>,
    pending: &mut Vec<render_model::ActorRigGeometry>,
    local: &mut render::ActorRigSubmission,
    animated: render::ActorRigSubmission,
) {
    if let Some(geometry) = rig.skin_geometry {
        let Some(id) = cache.rig(geometry, |built| {
            if let Some(equipment) = equipment.as_deref_mut() {
                equipment.register_skin_rig(built.id, rig.bone_names.to_vec());
            }
            pending.push(built);
        }) else {
            return;
        };
        local.input.rig = id;
    }
    // Keep placement/materials and the canonical first-person hand unchanged.
    local.input.previous_bones = animated.input.previous_bones;
    local.input.current_bones = animated.input.current_bones;
}
