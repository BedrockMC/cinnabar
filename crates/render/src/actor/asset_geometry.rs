//! Rig geometry built from the runtime entity catalog: bone merging, names, and pivots.
use std::sync::Arc;

use assets::{EntityGeometryBone, RuntimeEntityAssets, validate_entity_geometry_inheritance};

use super::{
    ActorRigGeometry, ActorRigGeometryError, EntityRigId, MAX_ACTOR_RIG_VERTICES,
    MAX_RENDER_BONES_PER_ACTOR,
};

pub(super) fn geometry_from_runtime_assets(
    assets: &RuntimeEntityAssets,
    binding_index: usize,
) -> Result<ActorRigGeometry, ActorRigGeometryError> {
    let binding = assets
        .rig_geometries()
        .get(binding_index)
        .ok_or(ActorRigGeometryError::InvalidAssetGeometry)?;
    geometry_from_geometry_index(
        assets,
        binding.geometry as usize,
        EntityRigId(
            u32::try_from(binding_index)
                .map_err(|_| ActorRigGeometryError::InvalidAssetGeometry)?,
        ),
    )
}

pub(super) fn geometry_from_geometry_index(
    assets: &RuntimeEntityAssets,
    geometry_index: usize,
    id: EntityRigId,
) -> Result<ActorRigGeometry, ActorRigGeometryError> {
    let bones = resolve_geometry_bones(assets, geometry_index)?;
    if bones.is_empty() || bones.len() > MAX_RENDER_BONES_PER_ACTOR {
        return Err(ActorRigGeometryError::BoneCount);
    }
    let mut vertices = Vec::new();
    for (bone_index, bone) in bones.iter().enumerate() {
        if bone.never_render == Some(true) {
            continue;
        }
        for cube in &bone.cubes {
            super::geometry::append_entity_cube_vertices(
                &mut vertices,
                cube,
                bone_index as u32,
                assets
                    .geometries()
                    .get(geometry_index)
                    .map(|geometry| (geometry.texture_width, geometry.texture_height))
                    .ok_or(ActorRigGeometryError::InvalidAssetGeometry)?,
                bone.mirror.unwrap_or(false),
                bone.inflate.map_or(0.0, |inflate| inflate.get()),
            )?;
            if vertices.len() > MAX_ACTOR_RIG_VERTICES {
                return Err(ActorRigGeometryError::CatalogCapacity);
            }
        }
    }
    let bone_pivots = bones.iter().map(bone_bind_pivot).collect::<Vec<_>>();
    ActorRigGeometry::new(id, Arc::from(vertices), Arc::from(bone_pivots))
}

/// Bone names of a geometry in rig order, after inheritance is merged.
#[must_use]
pub fn geometry_bone_names(
    assets: &RuntimeEntityAssets,
    geometry_index: usize,
) -> Option<Vec<Box<str>>> {
    resolve_geometry_bones(assets, geometry_index)
        .ok()
        .map(|bones| bones.into_iter().map(|bone| bone.name).collect())
}

/// Bind pivot (rig frame, blocks) of every bone of a geometry, in rig order.
#[must_use]
pub fn geometry_bone_pivots(
    assets: &RuntimeEntityAssets,
    geometry_index: usize,
) -> Option<Vec<[f32; 3]>> {
    resolve_geometry_bones(assets, geometry_index)
        .ok()
        .map(|bones| bones.iter().map(bone_bind_pivot).collect())
}

/// Index of the geometry with this identifier in the entity catalog.
#[must_use]
pub fn find_geometry_index(assets: &RuntimeEntityAssets, identifier: &str) -> Option<u32> {
    assets
        .geometries()
        .iter()
        .position(|geometry| geometry.identifier.as_ref() == identifier)
        .and_then(|index| u32::try_from(index).ok())
}

fn bone_bind_pivot(bone: &EntityGeometryBone) -> [f32; 3] {
    // Pivots share the vertices' rig frame, where authored X is mirrored.
    bone.pivot.map_or([0.0; 3], |pivot| {
        [
            -pivot[0].get() / 16.0,
            pivot[1].get() / 16.0,
            pivot[2].get() / 16.0,
        ]
    })
}

fn resolve_geometry_bones(
    assets: &RuntimeEntityAssets,
    geometry_index: usize,
) -> Result<Vec<EntityGeometryBone>, ActorRigGeometryError> {
    let parents = validate_entity_geometry_inheritance(assets.geometries())
        .map_err(|_| ActorRigGeometryError::InvalidAssetGeometry)?;
    let mut chain = Vec::new();
    let mut current = geometry_index;
    for _ in 0..=parents.len() {
        chain.push(current);
        let Some(parent) = parents.get(current).copied().flatten() else {
            break;
        };
        current = parent;
    }
    if chain
        .last()
        .and_then(|index| parents.get(*index))
        .copied()
        .flatten()
        .is_some()
    {
        return Err(ActorRigGeometryError::InvalidAssetGeometry);
    }
    chain.reverse();
    let mut merged: Vec<EntityGeometryBone> = Vec::new();
    for index in chain {
        for child in assets
            .geometries()
            .get(index)
            .ok_or(ActorRigGeometryError::InvalidAssetGeometry)?
            .bones
            .iter()
        {
            if let Some(existing) = merged
                .iter_mut()
                .find(|bone| bone.name.eq_ignore_ascii_case(&child.name))
            {
                overlay_geometry_bone(existing, child);
            } else {
                merged.push(child.clone());
            }
        }
    }
    Ok(merged)
}

fn overlay_geometry_bone(base: &mut EntityGeometryBone, child: &EntityGeometryBone) {
    if child.parent.is_some() {
        base.parent.clone_from(&child.parent);
    }
    if child.pivot.is_some() {
        base.pivot = child.pivot;
    }
    if child.rotation.is_some() {
        base.rotation = child.rotation;
    }
    if child.mirror.is_some() {
        base.mirror = child.mirror;
    }
    if child.inflate.is_some() {
        base.inflate = child.inflate;
    }
    if child.never_render.is_some() {
        base.never_render = child.never_render;
    }
    if child.reset.is_some() {
        base.reset = child.reset;
    }
    // `reset` drops the cubes a bone inherited; it is how derived geometries hide a bone.
    if child.reset == Some(true) {
        base.cubes = Box::default();
    }
    if !child.cubes.is_empty() {
        base.cubes.clone_from(&child.cubes);
    }
}

#[cfg(test)]
mod tests {
    use assets::{EntityGeometryBone, EntityGeometryCube, EntityGeometryScalar, EntityGeometryUv};

    use super::overlay_geometry_bone;

    fn bone(reset: Option<bool>, cubes: usize) -> EntityGeometryBone {
        let zero = EntityGeometryScalar::ZERO;
        let cube = EntityGeometryCube {
            origin: [zero; 3],
            size: [zero; 3],
            pivot: [zero; 3],
            rotation: [zero; 3],
            uv: EntityGeometryUv::Box([zero; 2]),
            inflate: zero,
            mirror: false,
        };
        EntityGeometryBone {
            name: "body".into(),
            parent: None,
            pivot: None,
            rotation: None,
            mirror: None,
            inflate: None,
            never_render: None,
            reset,
            cubes: vec![cube; cubes].into(),
        }
    }

    #[test]
    fn reset_drops_inherited_cubes_but_a_plain_overlay_keeps_them() {
        let mut kept = bone(None, 2);
        overlay_geometry_bone(&mut kept, &bone(None, 0));
        assert_eq!(kept.cubes.len(), 2);

        let mut hidden = bone(None, 2);
        overlay_geometry_bone(&mut hidden, &bone(Some(true), 0));
        assert!(hidden.cubes.is_empty());

        let mut replaced = bone(None, 2);
        overlay_geometry_bone(&mut replaced, &bone(Some(true), 1));
        assert_eq!(replaced.cubes.len(), 1);
    }
}
