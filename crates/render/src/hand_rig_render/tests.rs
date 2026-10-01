use super::*;
use crate::{ActorGpuInstance, ActorRigGeometrySpan, ActorRigVertex};

fn single_instance_frame() -> ActorRigRenderFrame {
    ActorRigRenderFrame {
        frame_generation: 1,
        geometry_revision: 1,
        instances: Arc::from([ActorGpuInstance::default()]),
        previous_bones: Arc::from([[[0.0; 4]; 3]]),
        current_bones: Arc::from([[[0.0; 4]; 3]]),
        geometry_vertices: crate::actor::ActorRigVertexSegments::from_vertices([
            ActorRigVertex::default(),
        ]),
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

/// A per-frame pose rewrites the same buffers instead of reallocating them and the bind group.
#[test]
fn pose_updates_reuse_their_buffers() {
    use bevy::{
        ecs::system::RunSystemOnce,
        render::renderer::{RenderDevice, RenderQueue, WgpuWrapper},
    };
    use std::{
        future::Future,
        pin::pin,
        task::{Context, Poll, Waker},
    };
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true },
            ..Default::default()
        },
        ..Default::default()
    });
    let mut context = Context::from_waker(Waker::noop());
    let Poll::Ready(Ok(adapter)) =
        pin!(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).poll(&mut context)
    else {
        panic!("noop adapter must be immediate");
    };
    let Poll::Ready(Ok((device, queue))) =
        pin!(adapter.request_device(&wgpu::DeviceDescriptor::default())).poll(&mut context)
    else {
        panic!("noop device must be immediate");
    };
    let device = RenderDevice::from(device);
    let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
    let mut world = bevy::prelude::World::new();
    world.insert_resource(device.clone());
    world.run_system_once(init_gpu).unwrap();
    let mut gpu = world.remove_resource::<HandRigGpu>().unwrap();
    let mut scene = HandRigScene::default();
    let mut buffers = Vec::new();
    for revision in 1..=3 {
        assert!(scene.publish(single_instance_frame(), skin(), light(), 1.2, revision));
        upload_pose(&mut gpu, &device, &queue, scene.frame.as_ref().unwrap());
        buffers.push(gpu.instances.as_ref().unwrap().id());
    }
    assert!(buffers.windows(2).all(|pair| pair[0] == pair[1]));
}
