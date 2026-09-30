use std::sync::Arc;

use protocol::{NetworkItemStack, VerifiedNetworkItemStack};
use sha2::{Digest, Sha256};

use super::*;

const BOW: i32 = 300;

fn selection(slot: u8, network_id: i32) -> FrozenMiningSelection {
    let extra_data: Arc<[u8]> = Arc::from([]);
    let stack = NetworkItemStack {
        network_id,
        metadata: 0,
        stack_network_id: 41,
        count: 1,
        nbt_digest: Sha256::digest(&extra_data).into(),
        block_runtime_id: 0,
        extra_data,
    };
    FrozenMiningSelection {
        slot,
        item: VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap(),
    }
}

fn frame(tick: u64, held: bool) -> UseFrame {
    UseFrame {
        tick,
        position: [0.5, 65.62, 0.5],
        held,
        selection: Some(selection(2, BOW)),
        air_use: classify("minecraft:bow", false, 0),
        has_ammo: true,
        press_consumed: false,
    }
}

/// Each packet as "use", "release" or "complete".
fn kinds(outcome: &UseOutcome) -> Vec<&'static str> {
    outcome
        .packets
        .iter()
        .map(|packet| {
            let debug = format!("{:?}", packet.data);
            if debug.contains("ItemUseInventoryTransaction(") {
                "use"
            } else if debug.contains("action_type: Release") {
                "release"
            } else if debug.contains("ItemReleaseInventoryTransaction(") {
                "complete"
            } else {
                "other"
            }
        })
        .collect()
}

#[test]
fn crossbow_charge_follows_quick_charge_and_a_loaded_one_fires() {
    let hold = |ticks| AirUse::Hold {
        max_ticks: ticks,
        ammo: Ammo::ArrowOrOffhandRocket,
    };
    assert_eq!(classify("minecraft:crossbow", false, 0), Some(hold(25)));
    assert_eq!(classify("minecraft:crossbow", false, 3), Some(hold(10)));
    assert_eq!(classify("minecraft:crossbow", false, 9), Some(hold(0)));
    assert_eq!(
        classify("minecraft:crossbow", true, 0),
        Some(AirUse::Instant)
    );
    assert_eq!(classify("minecraft:shield", false, 0), None);
}

/// A bow press sends click-air and starts a use; button-up sends one release.
#[test]
fn bow_press_starts_a_use_and_button_up_releases_it() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let started = runtime.step(&frame(100, true));
    assert_eq!(kinds(&started), ["use"]);
    assert!(started.started && runtime.is_using());

    let holding = runtime.step(&frame(120, true));
    assert!(holding.packets.is_empty() && runtime.is_using());

    let released = runtime.step(&frame(130, false));
    assert_eq!(kinds(&released), ["release"]);
    assert!(!runtime.is_using());
    assert!(runtime.step(&frame(131, false)).packets.is_empty());
}

/// An uncharged crossbow completes on its own once its charge duration runs out.
#[test]
fn crossbow_completes_when_its_charge_duration_depletes() {
    let crossbow = |tick| UseFrame {
        air_use: classify("minecraft:crossbow", false, 0),
        ..frame(tick, true)
    };
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    assert!(runtime.step(&crossbow(10)).started);
    assert!(runtime.step(&crossbow(34)).packets.is_empty());
    assert_eq!(kinds(&runtime.step(&crossbow(35))), ["complete"]);
    assert!(!runtime.is_using());
}

/// Without ammunition vanilla still sends click-air, but no use starts or releases.
#[test]
fn a_bow_without_arrows_sends_click_air_but_never_starts() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let outcome = runtime.step(&UseFrame {
        has_ammo: false,
        ..frame(100, true)
    });
    assert_eq!(kinds(&outcome), ["use"]);
    assert!(!outcome.started && !runtime.is_using());
    assert!(runtime.step(&frame(101, false)).packets.is_empty());
}

/// Reselecting stops the use without a release; a consumed press starts nothing.
#[test]
fn switching_slot_stops_silently_and_consumed_presses_do_nothing() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    runtime.step(&frame(100, true));
    let switched = runtime.step(&UseFrame {
        selection: Some(selection(3, BOW)),
        ..frame(101, true)
    });
    assert!(switched.packets.is_empty() && !runtime.is_using());

    runtime.observe_press(true);
    let consumed = runtime.step(&UseFrame {
        press_consumed: true,
        ..frame(102, true)
    });
    assert!(consumed.packets.is_empty() && !runtime.is_using());
}

/// A briefly unverifiable selection (inventory request in flight) keeps the use and its release.
#[test]
fn an_unverified_selection_keeps_the_use_and_still_releases() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    runtime.step(&frame(100, true));
    let hidden = |held| UseFrame {
        selection: None,
        ..frame(101, held)
    };
    assert!(runtime.step(&hidden(true)).packets.is_empty() && runtime.is_using());
    assert_eq!(kinds(&runtime.step(&hidden(false))), ["release"]);
}

/// Held Use without a fresh press never starts a use.
#[test]
fn held_use_without_a_press_starts_nothing() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(false);
    assert!(runtime.step(&frame(100, true)).packets.is_empty());
    assert!(!runtime.is_using());
}
