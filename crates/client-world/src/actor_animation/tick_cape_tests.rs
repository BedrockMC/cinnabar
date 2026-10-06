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

#[test]
fn local_flight_freezes_cape_walk_phase_but_keeps_chase_and_resumes_walking() {
    let mut compiled = super::super::attachable::tests::compiled_fixture();
    compiled.sources[1].path = "entity/player.json".into();
    compiled.symbols[4].kind = assets::EntityAssetKind::Entity;
    compiled.symbols[4].identifier = "minecraft:player".into();
    compiled.symbols.rotate_right(1);
    compiled.rig_bindings[0].entity_symbol = 0;
    compiled.rig_bindings[0].render_controller = 3;
    compiled.animation_clips[0].symbol = 2;
    let assets = Arc::new(assets::RuntimeEntityAssets::from_compiled(compiled).unwrap());
    let mut store = crate::actor_store::ActorStore::new_with_entity_assets(1, 0, assets);
    store.exclude_remote_state_for(1);
    let mut feed = crate::LocalPlayerFeed {
        prefer_client_skin: false,
        uuid: [1; 16],
        username: "Player".into(),
        skin: protocol::PlayerSkin::Unavailable(protocol::PlayerSkinUnavailable::InvalidDimensions),
        position: [0.0, 64.0, 0.0],
        velocity: [0.2, 0.0, 0.0],
        on_ground: true,
        flying: false,
        yaw: 90.0,
        head_yaw: 90.0,
        pitch: 0.0,
        main_hand: None,
        off_hand: None,
        main_hand_metadata: 0,
        main_hand_stack_id: None,
        main_hand_slot: 0,
        java_swing_ticks: crate::ACTOR_SWING_TICKS,
        teleported: false,
        first_person: false,
        view_bobbing: true,
        sneaking: false,
        sprinting: false,
        item_use: Default::default(),
    };
    let step = |store: &mut crate::actor_store::ActorStore, feed: &crate::LocalPlayerFeed| {
        store.sync_local_player(1, -1, feed);
        store.advance_interpolation_ticks(1);
        store.actor_rig(1).unwrap().java
    };
    step(&mut store, &feed);
    feed.position[0] = 0.2;
    let walking = step(&mut store, &feed);
    assert!(walking.walked[1] > 0.0);
    feed.flying = true;
    feed.on_ground = false;
    feed.position[0] = 0.4;
    let flying = step(&mut store, &feed);
    assert_eq!(flying.walked, [walking.walked[1]; 2]);
    assert_ne!(
        flying.cape[1], walking.cape[1],
        "flight still advances inertia"
    );
    assert!(flying.limb_swing[1] > walking.limb_swing[1]);
    feed.position[0] = 0.6;
    let flying = step(&mut store, &feed);
    assert_eq!(flying.walked, [walking.walked[1]; 2]);
    feed.flying = false;
    feed.on_ground = true;
    feed.position[0] = 0.8;
    let resumed = step(&mut store, &feed);
    assert_eq!(resumed.walked[0], walking.walked[1]);
    assert!((resumed.walked[1] - walking.walked[1] - 0.12).abs() < 1e-6);
}

#[test]
fn rig_snapshot_retains_java_equip_and_native_wing_inputs() {
    let (mut actor, mut store) = fixture();
    actor.on_ground = Some(false);
    actor
        .metadata
        .insert(0, ActorMetadataValue::Flags(1 << query::FLAG_GLIDING));
    let context = ActorTickContext {
        main_hand: Some(Arc::from("minecraft:potion")),
        main_hand_metadata: 21,
        has_cape: true,
        armor: [
            None,
            Some(WornArmor {
                item: "minecraft:elytra".into(),
                dye_rgb: None,
            }),
            None,
            None,
            None,
        ],
        ..Default::default()
    };
    for tick in 0..3 {
        actor.position = [tick as f32 * 0.2, tick as f32 * -0.4, 0.0];
        advance(&actor, &mut store, context.clone());
    }
    let rig = store.get(actor.runtime_id).unwrap();
    let input = rig
        .animation_variables
        .input()
        .expect("worn item owner inputs");
    assert_eq!(input.position_delta, [0.2, -0.4, 0.0]);
    assert_eq!(rig.java_equipped.unwrap().metadata, 21);
    assert!(
        rig.java.vanilla_posture,
        "gliding retains its authored body pose"
    );
    assert_ne!(rig.java.cape[0], rig.java.cape[1]);
}
