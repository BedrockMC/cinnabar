use super::*;
use crate::{ActorGpuInstance, ActorRigGeometrySpan, ActorRigVertex};

fn single_instance_frame() -> ActorRigRenderFrame {
    ActorRigRenderFrame {
        frame_generation: 1,
        geometry_revision: 1,
        instances: Arc::from([ActorGpuInstance::default()]),
        previous_bones: Arc::from([[[0.0; 4]; 3]]),
        current_bones: Arc::from([[[0.0; 4]; 3]]),
        geometry_vertices: Arc::from([ActorRigVertex::default()]),
        geometry_spans: Arc::from([ActorRigGeometrySpan {
            first_vertex: 0,
            vertex_count: 1,
        }]),
        manifest: Arc::from([]),
        maximum_vertex_count: 36,
        rejects: crate::ActorRigRejects::default(),
    }
}

fn skin() -> Arc<[u8]> {
    Arc::from(vec![0u8; crate::STANDARD_SKIN_BYTES])
}

fn light() -> HandRigLight {
    HandRigLight {
        block_level: 15,
        sky_level: 0,
        daylight: 1.0,
        pad: 0,
    }
}

#[test]
fn publish_accepts_a_single_instance_lit_rig_and_activates() {
    let mut scene = HandRigScene::default();
    assert!(scene.publish(single_instance_frame(), skin(), light(), 1.2, 7));
    assert!(scene.is_active());
}

#[test]
fn publish_rejects_a_wrong_sized_skin_and_stays_inactive() {
    let mut scene = HandRigScene::default();
    let bad_skin = Arc::from(vec![0u8; 10]);
    assert!(!scene.publish(single_instance_frame(), bad_skin, light(), 1.2, 7));
    assert!(!scene.is_active());
}

#[test]
fn publish_rejects_non_finite_or_out_of_range_fov_and_zero_revision() {
    let mut scene = HandRigScene::default();
    assert!(!scene.publish(single_instance_frame(), skin(), light(), f32::NAN, 7));
    assert!(!scene.publish(single_instance_frame(), skin(), light(), 0.0, 7));
    assert!(!scene.publish(single_instance_frame(), skin(), light(), 4.0, 7));
    assert!(!scene.publish(single_instance_frame(), skin(), light(), 1.2, 0));
    assert!(!scene.is_active());
}

#[test]
fn publish_rejects_an_empty_rig_and_clears_a_prior_frame() {
    let mut scene = HandRigScene::default();
    assert!(scene.publish(single_instance_frame(), skin(), light(), 1.2, 7));
    assert!(!scene.publish(ActorRigRenderFrame::default(), skin(), light(), 1.2, 8));
    assert!(!scene.is_active());
}

#[test]
fn item_atlas_is_kept_only_when_its_pixel_count_matches_and_a_frame_is_active() {
    let atlas = |bytes: usize| HandItemAtlas {
        width: 2,
        height: 2,
        layers: 2,
        rgba8: Arc::from(vec![0u8; bytes]),
    };
    let mut scene = HandRigScene::default();
    scene.set_item_atlas(Some(atlas(32)));
    assert!(!scene.is_active());

    assert!(scene.publish(single_instance_frame(), skin(), light(), 1.2, 7));
    scene.set_item_atlas(Some(atlas(31)));
    assert!(scene.frame.as_ref().unwrap().item_atlas.is_none());
    scene.set_item_atlas(Some(atlas(32)));
    assert!(scene.frame.as_ref().unwrap().item_atlas.is_some());
}
