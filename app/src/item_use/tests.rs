use std::sync::Arc;

use protocol::{BedrockSession, NetworkItemStack, VerifiedNetworkItemStack};
use sha2::{Digest, Sha256};

use super::*;

const BOW: i32 = 300;
const SNOWBALL: i32 = 388;
const MENU_ITEM: i32 = 20329;

fn stack(slot: u8, network_id: i32, count: u16) -> FrozenMiningSelection {
    let extra_data: Arc<[u8]> = Arc::from([]);
    let stack = NetworkItemStack {
        network_id,
        metadata: 0,
        stack_network_id: 41,
        count,
        nbt_digest: Sha256::digest(&extra_data).into(),
        block_runtime_id: 0,
        extra_data,
    };
    FrozenMiningSelection {
        slot,
        item: VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap(),
    }
}

fn selection(slot: u8, network_id: i32) -> FrozenMiningSelection {
    stack(slot, network_id, 1)
}

fn frame(tick: u64, held: bool) -> UseFrame {
    UseFrame {
        tick,
        now_millis: tick * 50,
        position: [0.5, 65.62, 0.5],
        held,
        selection: Some(selection(2, BOW)),
        air_use: classify("minecraft:bow", false, 0),
        ready: true,
        creative: false,
        press_consumed: false,
    }
}

fn item_frame(
    tick: u64,
    held: bool,
    selection: FrozenMiningSelection,
    identifier: &str,
) -> UseFrame {
    UseFrame {
        selection: Some(selection),
        air_use: classify(identifier, false, 0),
        ..frame(tick, held)
    }
}

/// Each packet as "use" or "release".
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
            } else {
                "other"
            }
        })
        .collect()
}

/// The packet after an encode/decode round trip, in debug form.
fn wire(packet: &protocol::Packet) -> String {
    let session = BedrockSession { shield_item_id: 0 };
    let bytes = protocol::encode(packet, &session).unwrap();
    format!(
        "{:?}",
        protocol::decode_batch(bytes, &session).unwrap()[0].data
    )
}

/// Every value printed for `key`, in order.
fn values(debug: &str, key: &str) -> Vec<String> {
    let needle = format!("{key}: ");
    debug
        .match_indices(&needle)
        .map(|(start, _)| {
            let rest = &debug[start + needle.len()..];
            let mut depth = 0;
            let end = rest
                .char_indices()
                .find(|(_, c)| match c {
                    '(' | '[' | '{' => {
                        depth += 1;
                        false
                    }
                    ')' | ']' | '}' if depth > 0 => {
                        depth -= 1;
                        false
                    }
                    ',' | ')' | ']' | '}' => true,
                    _ => false,
                })
                .map_or(rest.len(), |(end, _)| end);
            rest[..end].trim().to_owned()
        })
        .collect()
}

/// The legacy request id, the action count, and every stack's size and net id (actions first).
fn summary(debug: &str) -> (String, usize, Vec<String>, Vec<String>) {
    let legacy = debug
        .split("legacy_request_id: ")
        .nth(1)
        .and_then(|rest| values(rest, "id").into_iter().next())
        .unwrap();
    (
        legacy,
        debug.matches("InventoryAction {").count(),
        values(debug, "stacksize"),
        values(debug, "net_id_variant"),
    )
}

#[test]
fn crossbow_charge_follows_quick_charge_and_a_loaded_one_fires() {
    let hold = |ticks| AirUse::Hold {
        max_ticks: ticks,
        needs: Needs::ArrowOrOffhandRocket,
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

#[test]
fn throwables_consume_one_and_pearls_and_wind_charges_cool_down() {
    for name in [
        "minecraft:snowball",
        "minecraft:egg",
        "minecraft:experience_bottle",
        "minecraft:splash_potion",
        "minecraft:lingering_potion",
    ] {
        assert_eq!(
            classify(name, false, 0),
            Some(AirUse::Throw { cooldown: None })
        );
    }
    let cooldown = |name| classify(name, false, 0).and_then(AirUse::cooldown);
    assert_eq!(
        cooldown("minecraft:ender_pearl").map(|cooldown| (cooldown.category, cooldown.ticks)),
        Some(("ender_pearl", 20))
    );
    assert_eq!(
        cooldown("minecraft:wind_charge").map(|cooldown| cooldown.ticks),
        Some(10)
    );
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

/// A depleted use completes locally: the client sends nothing (`Player::completeUsingItem`).
#[test]
fn a_depleted_crossbow_charge_ends_without_a_packet() {
    let crossbow = |tick| UseFrame {
        air_use: classify("minecraft:crossbow", false, 0),
        ..frame(tick, true)
    };
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    assert!(runtime.step(&crossbow(10)).started);
    assert!(runtime.step(&crossbow(34)).packets.is_empty());
    assert!(runtime.step(&crossbow(35)).packets.is_empty());
    assert!(!runtime.is_using());
    // A crossbow never repeats while held.
    assert!(runtime.step(&crossbow(60)).packets.is_empty());
}

/// Without ammunition vanilla still sends click-air, but no use starts or releases.
#[test]
fn a_bow_without_arrows_sends_click_air_but_never_starts() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let outcome = runtime.step(&UseFrame {
        ready: false,
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
        ..frame(110, true)
    });
    assert!(consumed.packets.is_empty() && !runtime.is_using());
    // Holding on after a consumed press never repeats.
    assert!(runtime.step(&frame(130, true)).packets.is_empty());
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

/// An accepted use slows movement input by vanilla's default factor until it ends.
#[test]
fn an_active_use_slows_movement_until_it_ends() {
    let mut runtime = ItemUseRuntime::default();
    assert_eq!(runtime.movement_modifier(), None);
    runtime.observe_press(true);
    runtime.step(&frame(100, true));
    assert_eq!(runtime.movement_modifier(), Some(0.35));
    runtime.step(&frame(110, false));
    assert_eq!(runtime.movement_modifier(), None);
}

/// A server menu item (no use behavior) sends a plain click-air on press and every 200 ms held.
#[test]
fn a_custom_menu_item_sends_click_air_and_repeats_while_held() {
    let menu = |tick, held| UseFrame {
        air_use: None,
        ..item_frame(tick, held, selection(0, MENU_ITEM), "zeqa:item.ffa")
    };
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let pressed = runtime.step(&menu(100, true));
    assert_eq!(kinds(&pressed), ["use"]);
    assert!(!pressed.started && !pressed.swung && !runtime.is_using());
    let debug = wire(&pressed.packets[0]);
    let (legacy, actions, _, _) = summary(&debug);
    assert_eq!((legacy.as_str(), actions), ("0", 0));
    assert_eq!(values(&debug, "slot"), ["0"]);
    assert!(debug.contains(&format!("id: {MENU_ITEM}")));

    // 200 ms re-arm: ticks 101..=104 are at most 200 ms later.
    for tick in 101..=104 {
        assert!(runtime.step(&menu(tick, true)).packets.is_empty());
    }
    assert_eq!(kinds(&runtime.step(&menu(105, true))), ["use"]);
    assert!(runtime.step(&menu(106, false)).packets.is_empty());
    assert!(runtime.step(&menu(120, false)).packets.is_empty());
}

/// A thrown snowball swings and reports the decrement; the next throw starts from the predicted
/// stack under the next legacy request id until the server restates the slot.
#[test]
fn a_snowball_throw_reports_its_predicted_decrement() {
    let snowballs = stack(4, SNOWBALL, 16);
    let throw = |tick, held| item_frame(tick, held, snowballs.clone(), "minecraft:snowball");
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let first = runtime.step(&throw(100, true));
    assert_eq!(kinds(&first), ["use"]);
    assert!(first.swung && !first.started);
    let debug = wire(&first.packets[0]);
    assert_eq!(
        summary(&debug),
        (
            "-4".to_owned(),
            1,
            vec!["16".to_owned(), "15".to_owned(), "16".to_owned()],
            vec![
                "Some(41)".to_owned(),
                "Some(-4)".to_owned(),
                "Some(41)".to_owned()
            ],
        )
    );
    assert!(debug.contains("container_enum: Inventorycontainer, slots: [4]"));
    assert!(debug.contains("source_type: Containerinventory, container_id: Some(0)"));

    let second = runtime.step(&throw(105, true));
    assert_eq!(
        summary(&wire(&second.packets[0])),
        (
            "-6".to_owned(),
            1,
            vec!["15".to_owned(), "14".to_owned(), "15".to_owned()],
            vec![
                "Some(-4)".to_owned(),
                "Some(-6)".to_owned(),
                "Some(-4)".to_owned()
            ],
        )
    );

    // The server restating the slot replaces the prediction.
    let restated = stack(4, SNOWBALL, 14);
    let third = runtime.step(&item_frame(110, true, restated, "minecraft:snowball"));
    let (_, _, sizes, ids) = summary(&wire(&third.packets[0]));
    assert_eq!((sizes[0].as_str(), ids[0].as_str()), ("14", "Some(41)"));
}

/// Creative throws swing but change no stack, so they carry no action or legacy request.
#[test]
fn a_creative_throw_reports_no_inventory_change() {
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let outcome = runtime.step(&UseFrame {
        creative: true,
        ..item_frame(100, true, stack(1, SNOWBALL, 16), "minecraft:snowball")
    });
    assert!(outcome.swung);
    let (legacy, actions, _, _) = summary(&wire(&outcome.packets[0]));
    assert_eq!((legacy.as_str(), actions), ("0", 0));
}

/// An ender pearl on cooldown still sends click-air, but neither swings nor consumes.
#[test]
fn an_ender_pearl_on_cooldown_sends_a_plain_click_air() {
    const PEARL: i32 = 422;
    let pearl = |tick, pressed_count| {
        item_frame(
            tick,
            false,
            stack(2, PEARL, pressed_count),
            "minecraft:ender_pearl",
        )
    };
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    let thrown = runtime.step(&pearl(100, 16));
    assert!(thrown.swung);
    runtime.observe_press(true);
    let cooling = runtime.step(&pearl(110, 15));
    assert_eq!(kinds(&cooling), ["use"]);
    assert!(!cooling.swung);
    assert_eq!(summary(&wire(&cooling.packets[0])).1, 0);
    runtime.observe_press(true);
    assert!(runtime.step(&pearl(120, 15)).swung);
}

/// `TypedClientNetId::_generateNext` restarts at -4 once the counter leaves the negative range.
#[test]
fn legacy_request_ids_step_down_by_two_and_wrap() {
    let mut runtime = ItemUseRuntime::default();
    assert_eq!(runtime.next_legacy_request_id(), -4);
    assert_eq!(runtime.next_legacy_request_id(), -6);
    runtime.last_legacy_request_id = i32::MIN;
    assert_eq!(runtime.next_legacy_request_id(), -4);
}
