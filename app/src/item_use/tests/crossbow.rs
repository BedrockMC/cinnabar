use super::*;

fn crossbow(tick: u64, held: bool) -> UseFrame {
    UseFrame {
        air_use: classify("minecraft:crossbow", false, 0, None),
        charge_projectile: Some("minecraft:arrow"),
        ..frame(tick, held)
    }
}

fn charged(runtime: &ItemUseRuntime, frame: &UseFrame) -> Option<Option<&'static str>> {
    runtime
        .crossbows
        .verified_projectile(frame.selection.as_ref()?, frame.inventory_revision)
}

fn load() -> ItemUseRuntime {
    let mut runtime = ItemUseRuntime::default();
    runtime.synchronize(1);
    runtime.observe_press(true);
    assert!(runtime.step(&crossbow(10, true)).started);
    assert!(runtime.step(&crossbow(35, true)).packets.is_empty());
    runtime
}

#[test]
fn full_charge_persists_until_another_press_fires_without_recharging() {
    let mut runtime = load();
    let idle = crossbow(36, true);
    assert_eq!(charged(&runtime, &idle), Some(Some("minecraft:arrow")));
    assert_eq!(runtime.crossbows.air_use(&idle), Some(AirUse::Instant));
    assert_eq!(
        crossbow_animation_frame(
            None,
            crossbow_duration(),
            charged(&runtime, &idle).flatten(),
            false
        ),
        4
    );
    assert!(!runtime.is_using());
    for held in [true, false, false] {
        assert!(runtime.step(&crossbow(37, held)).packets.is_empty());
    }
    runtime.observe_press(true);
    let fired = runtime.step(&crossbow(38, true));
    assert_eq!(kinds(&fired), ["use"]);
    assert!(!fired.started && !runtime.is_using());
    assert_eq!(charged(&runtime, &idle), Some(None));
    assert_eq!(
        crossbow_animation_frame(
            None,
            crossbow_duration(),
            charged(&runtime, &idle).flatten(),
            false
        ),
        0
    );
    // Prediction changes no outgoing verified inventory descriptor.
    let expected =
        protocol::click_air_packet(held_request(idle.selection.as_ref().unwrap(), &idle), None)
            .unwrap();
    let session = protocol::BedrockSession { shield_item_id: 0 };
    assert_eq!(
        protocol::encode(&fired.packets[0], &session).unwrap(),
        protocol::encode(&expected, &session).unwrap()
    );
    runtime.observe_press(true);
    assert!(runtime.step(&crossbow(44, true)).started);
}

#[test]
fn an_authoritative_restatement_even_identical_rejects_a_load_prediction() {
    let mut runtime = load();
    let corrected = UseFrame {
        inventory_revision: Some(2),
        ..crossbow(36, true)
    };
    assert_eq!(charged(&runtime, &corrected), None);
    assert_eq!(runtime.crossbows.air_use(&corrected), corrected.air_use);
    runtime.observe_press(true);
    assert!(runtime.step(&corrected).started);
}

#[test]
fn an_authoritative_loaded_state_wins_after_a_fire_correction() {
    let mut runtime = ItemUseRuntime::default();
    let authoritative_loaded = UseFrame {
        air_use: Some(AirUse::Instant),
        ..crossbow(10, true)
    };
    runtime.observe_press(true);
    assert!(!runtime.step(&authoritative_loaded).started);
    assert_eq!(charged(&runtime, &authoritative_loaded), Some(None));
    assert!(matches!(
        runtime.crossbows.air_use(&authoritative_loaded),
        Some(AirUse::Hold { .. })
    ));
    let corrected = UseFrame {
        inventory_revision: Some(2),
        ..authoritative_loaded
    };
    assert_eq!(charged(&runtime, &corrected), None);
    assert_eq!(runtime.crossbows.air_use(&corrected), Some(AirUse::Instant));
}

#[test]
fn early_release_cancels_but_release_at_depletion_loads() {
    for (tick, expected, projectile) in [
        (34, vec!["release"], None),
        (35, vec![], Some(Some("minecraft:arrow"))),
    ] {
        let mut runtime = ItemUseRuntime::default();
        runtime.observe_press(true);
        runtime.step(&crossbow(10, true));
        let released = crossbow(tick, false);
        assert_eq!(kinds(&runtime.step(&released)), expected);
        assert_eq!(charged(&runtime, &released), projectile);
    }
}

#[test]
fn loading_uses_the_projectile_present_when_charge_completes() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    runtime.step(&crossbow(10, true));
    let completed = UseFrame {
        charge_projectile: Some("minecraft:firework_rocket"),
        ..crossbow(35, true)
    };
    runtime.step(&completed);
    assert_eq!(
        charged(&runtime, &completed),
        Some(Some("minecraft:firework_rocket"))
    );
    assert_eq!(
        crossbow_animation_frame(
            None,
            crossbow_duration(),
            charged(&runtime, &completed).flatten(),
            false
        ),
        5
    );
}

#[test]
fn switching_away_preserves_the_loaded_stack_but_not_a_partial_charge() {
    let mut runtime = load();
    let switched = UseFrame {
        selection: Some(selection(3, BOW)),
        ..crossbow(36, true)
    };
    assert_eq!(charged(&runtime, &switched), None);
    assert!(runtime.step(&switched).packets.is_empty());
    assert_eq!(
        charged(&runtime, &crossbow(37, false)),
        Some(Some("minecraft:arrow"))
    );
    let mut partial = ItemUseRuntime::default();
    partial.observe_press(true);
    partial.step(&crossbow(10, true));
    assert!(partial.step(&switched).packets.is_empty());
    assert!(!partial.is_using());
    assert_eq!(charged(&partial, &crossbow(37, false)), None);
}

#[test]
fn a_new_stack_identity_or_session_cannot_inherit_a_charge() {
    let mut runtime = load();
    let mut new_stack = selection(2, BOW);
    let extra_data: Arc<[u8]> = Arc::from([]);
    let changed = NetworkItemStack {
        network_id: BOW,
        metadata: 0,
        stack_network_id: 42,
        count: 1,
        nbt_digest: Sha256::digest(&extra_data).into(),
        block_runtime_id: 0,
        extra_data,
    };
    new_stack.item =
        VerifiedNetworkItemStack::try_new(changed.clone(), changed.nbt_digest).unwrap();
    assert_eq!(
        charged(
            &runtime,
            &UseFrame {
                selection: Some(new_stack),
                ..crossbow(36, true)
            }
        ),
        None
    );
    runtime.synchronize(2);
    assert_eq!(charged(&runtime, &crossbow(36, true)), None);
    assert!(!runtime.has_work(false));
}
