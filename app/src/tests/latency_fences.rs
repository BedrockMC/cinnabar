//! Reuses the committed-authority harness to check movement/probe ordering.

use super::*;
use crate::{movement::MovementSource, runtime::network::NetworkHandle};
use sim::{Aabb, CollisionQuery, CollisionWorld, MovementInput, WorldQueryError};
use std::time::Duration;

struct EmptyWorld;

impl CollisionWorld for EmptyWorld {
    /// Supplies empty space for the isolated impulse ordering witness.
    fn collision_boxes(&self, _: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        Ok(CollisionQuery::synthetic(Vec::new()))
    }
}

/// Admits completed movement to the actual outbound command FIFO.
fn flush_inputs(app: &mut App) {
    let mut ticker = app.world_mut().remove_resource::<MovementTicker>().unwrap();
    let network = app.world().resource::<NetworkHandle>();
    crate::movement::flush_player_auth_inputs(&mut ticker, 8, None, |identity, packet| {
        network.send_physics_packet(identity, packet, None)
    })
    .unwrap();
    app.insert_resource(ticker);
}

#[test]
fn old_input_precedes_motion_fence_and_new_input_follows_it() {
    let mut app = app();
    let (network, mut packets) = NetworkHandle::stub_capturing_packets();
    app.insert_resource(network);
    let anchor = [0.0, 70.0, 0.0];
    let sample = {
        let mut physics = app.world_mut().resource_mut::<LocalPhysicsController>();
        physics.reanchor_network_position(anchor, 0, false);
        physics
            .advance(
                Duration::from_millis(50),
                MovementInput::default(),
                &EmptyWorld,
            )
            .samples
            .remove(0)
    };
    let old_velocity = app
        .world()
        .resource::<LocalPhysicsController>()
        .state()
        .unwrap()
        .velocity;
    {
        let mut ticker = app.world_mut().resource_mut::<MovementTicker>();
        ticker.reset(1, 0, anchor);
        ticker.set_source(MovementSource::Physics);
        ticker.enqueue_completed_physics(sample).unwrap();
    }
    {
        let mut world = app.world_mut().resource_mut::<ClientWorld>();
        let stream = world.stream.as_mut().unwrap();
        stream
            .submit(
                1,
                WorldEvent::ActorMotion(protocol::ActorMotionEvent {
                    actor_runtime_id: 42,
                    motion: [0.0, 0.4, 0.0],
                    tick: 0,
                }),
            )
            .unwrap();
        stream
            .submit(2, WorldEvent::NetworkStackLatency(777))
            .unwrap();
    }
    app.world_mut()
        .run_system_once(reconcile_world_stream_before_physics)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<LocalPhysicsController>()
            .state()
            .unwrap()
            .velocity,
        old_velocity
    );
    assert!(packets.drain().is_empty());
    assert!(
        !app.world()
            .resource::<MovementTicker>()
            .can_advance_physics_frame()
    );
    assert!(
        app.world()
            .resource::<MovementTicker>()
            .accepting_physics_admissions()
    );
    flush_inputs(&mut app);
    let sent = packets.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        protocol::player_auth_input_trace_sample(&sent[0])
            .unwrap()
            .tick,
        1
    );

    app.world_mut()
        .run_system_once(reconcile_world_stream_before_physics)
        .unwrap();
    assert_eq!(
        app.world()
            .resource::<LocalPhysicsController>()
            .state()
            .unwrap()
            .velocity
            .y,
        f64::from(0.4_f32)
    );
    assert!(
        app.world()
            .resource::<MovementTicker>()
            .can_advance_physics_frame()
    );
    let echoes = packets.drain();
    let session = protocol::BedrockSession { shield_item_id: 0 };
    assert_eq!(echoes.len(), 1);
    assert_eq!(
        protocol::encode(&echoes[0], &session).unwrap(),
        protocol::encode(&protocol::network_stack_latency_reply(777), &session).unwrap()
    );
    let sample = app
        .world_mut()
        .resource_mut::<LocalPhysicsController>()
        .advance(
            Duration::from_millis(50),
            MovementInput::default(),
            &EmptyWorld,
        )
        .samples
        .remove(0);
    assert_eq!(sample.movement[1], 0.4);
    app.world_mut()
        .resource_mut::<MovementTicker>()
        .enqueue_completed_physics(sample)
        .unwrap();
    flush_inputs(&mut app);
    let sent = packets.drain();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        protocol::player_auth_input_trace_sample(&sent[0])
            .unwrap()
            .tick,
        2
    );
}
