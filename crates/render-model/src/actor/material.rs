//! Native material contracts missing from the current entity carrier.
use assets::{EntityAssetKind, RuntimeEntityAssets};

use super::super::{ActorRigVertex, ONE_SIDED_BACK_UV};

/// 26.50 vanilla `arrow:entity_alphatest` inherits `entity_nocull`'s DisableCulling.
/// The native shader samples the same UV on both sides of each authored quad.
/// General authored render-controller material propagation remains incomplete; this
/// classification is restricted to geometry bound to the vanilla arrow actor.
pub(super) fn apply_native_arrow_material(
    assets: &RuntimeEntityAssets,
    geometry: usize,
    vertices: &mut [ActorRigVertex],
) {
    let is_arrow = assets.rig_bindings().iter().any(|rig| {
        assets
            .symbols()
            .get(rig.entity_symbol as usize)
            .is_some_and(|symbol| {
                symbol.kind == EntityAssetKind::Entity
                    && symbol.identifier.as_ref() == "minecraft:arrow"
            })
            && assets
                .rig_geometries()
                .get(
                    rig.first_geometry as usize
                        ..rig.first_geometry as usize + usize::from(rig.geometry_count),
                )
                .is_some_and(|bindings| {
                    bindings
                        .iter()
                        .any(|binding| binding.geometry as usize == geometry)
                })
    });
    if is_arrow {
        disable_planar_culling(vertices);
    }
}

fn disable_planar_culling(vertices: &mut [ActorRigVertex]) {
    for vertex in vertices {
        if vertex.back_uv == ONE_SIDED_BACK_UV {
            vertex.back_uv = vertex.uv;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_nocull_samples_the_authored_face_uv_from_both_sides() {
        let mut vertices = [
            ActorRigVertex {
                uv: [0.5, 5.0 / 32.0],
                back_uv: ONE_SIDED_BACK_UV,
                ..ActorRigVertex::default()
            },
            ActorRigVertex {
                uv: [0.25, 0.125],
                back_uv: [0.75, 0.375],
                ..ActorRigVertex::default()
            },
        ];
        disable_planar_culling(&mut vertices);
        assert_eq!(vertices[0].back_uv, vertices[0].uv);
        assert_eq!(vertices[1].back_uv, [0.75, 0.375]);
    }
}
