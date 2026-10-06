use super::*;

fn fixture() -> (ActorSnapshot, ActorAnimationStore) {
    let mut actor = super::super::tests::actor_with_metadata(HashMap::new());
    actor.on_ground = Some(true);
    let assets = super::super::render_frame::tests::counting_random_assets();
    let mut store = ActorAnimationStore::with_assets(assets);
    store.insert(1, 0, &actor);
    (actor, store)
}

fn advance(
    actor: &ActorSnapshot,
    store: &mut ActorAnimationStore,
    context: ActorTickContext,
) -> super::super::java::JavaMotion {
    store.advance_tick(
        &HashMap::from([(actor.runtime_id, actor.clone())]),
        None,
        None,
        false,
        true,
        |_| context.clone(),
    );
    store.get(actor.runtime_id).unwrap().java
}

#[test]
fn cape_bob_reads_native_motion_and_decays_after_remote_interpolation_clears_it() {
    let (mut actor, mut store) = fixture();
    actor.velocity = [5.0, 0.0, 0.0];
    actor.status.native_velocity = [0.025, 0.0, 0.0];
    let motion = advance(&actor, &mut store, ActorTickContext::default());
    assert!((motion.bob[1] - 0.01).abs() < 1e-8);
    actor.status.native_velocity = [0.0; 3];
    let motion = advance(&actor, &mut store, ActorTickContext::default());
    assert!((motion.bob[1] - 0.006).abs() < 1e-8);
}

#[test]
fn cape_bob_decays_when_the_actor_is_dead_or_has_zero_health() {
    for dead_status in [false, true] {
        let (mut actor, mut store) = fixture();
        actor.status.native_velocity = [0.1, 0.0, 0.0];
        actor.velocity = actor.status.native_velocity;
        let motion = advance(&actor, &mut store, ActorTickContext::default());
        assert!((motion.bob[1] - 0.04).abs() < 1e-8);
        if dead_status {
            actor.status.dead = true;
        } else {
            actor.attributes.insert(
                "minecraft:health".into(),
                protocol::ActorAttribute {
                    name: "minecraft:health".into(),
                    min: 0.0,
                    max: 20.0,
                    current: 0.0,
                    default: None,
                    modifiers: Arc::from([]),
                },
            );
        }
        let motion = advance(&actor, &mut store, ActorTickContext::default());
        assert!((motion.bob[1] - 0.024).abs() < 1e-8);
    }
}

#[test]
fn cape_bob_clears_immediately_while_riding() {
    let (mut actor, mut store) = fixture();
    actor.velocity = [0.1, 0.0, 0.0];
    actor.status.native_velocity = actor.velocity;
    let motion = advance(&actor, &mut store, ActorTickContext::default());
    assert!((motion.bob[1] - 0.04).abs() < 1e-8);
    let previous_bob = motion.bob[1];
    let motion = advance(
        &actor,
        &mut store,
        ActorTickContext {
            is_riding: true,
            ..Default::default()
        },
    );
    assert_eq!(motion.bob, [previous_bob, 0.0]);
}
