//! Unthrottled movement authority records under the outbound trace switch.

use client_world::CommittedControlEvent;
use serde_json::json;

use super::{LocalPhysicsController, MovementTicker, trace};

/// Records incoming movement state before a correction can rewrite history.
pub(crate) fn trace_server_control(
    ticker: &MovementTicker,
    physics: &LocalPhysicsController,
    control: &CommittedControlEvent,
) {
    if !trace::movement_trace_enabled() {
        return;
    }
    let mut record = match control {
        CommittedControlEvent::PlayerMovementCorrection { correction, .. } => {
            let sent = ticker.sent_history.iter().find(|sent| {
                sent.session_generation == ticker.session_generation && sent.tick == correction.tick
            });
            let retained = physics.sample_at(correction.tick);
            json!({
                "kind": "correct", "tick": correction.tick,
                "position": correction.position, "velocity": correction.delta,
                "on_ground": correction.on_ground,
                "pitch": correction.pitch, "yaw": correction.yaw,
                "sent_position": sent.map(|sent| sent.position),
                "retained_position": retained.map(|sample| sample.position),
                "retained_velocity": retained.map(|sample| sample.velocity),
                "retained_on_ground": retained.map(|sample| sample.grounded_after_tick),
            })
        }
        CommittedControlEvent::NetworkStackLatency {
            sequence,
            creation_time,
        } => json!({
            "kind": "latency", "sequence": sequence, "creation_time": creation_time,
        }),
        CommittedControlEvent::LocalActorMotion { event, .. } => json!({
            "kind": "motion", "tick": event.tick, "velocity": event.motion,
        }),
        CommittedControlEvent::MovePlayer { movement, .. } => json!({
            "kind": "move_player", "tick": movement.source_tick,
            "position": movement.position, "pitch": movement.pitch, "yaw": movement.yaw,
            "on_ground": movement.on_ground, "teleported": movement.mode.is_teleport(),
        }),
        _ => return,
    };
    record["schema"] = json!("rust-mcbe-movement-control-v1");
    record["session_generation"] = json!(ticker.session_generation);
    record["local_tick"] = json!(physics.state().map(|state| state.tick));
    record["next_input_tick"] = json!(ticker.next_tick);
    trace::write_trace_line(&record.to_string());
}
