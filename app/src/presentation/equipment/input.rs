//! Gathers what an actor wears and holds from the world stream (and, for the local player, the
//! client-owned inventory) into runtime input.

use assets::ItemVisualRoute;
use client_world::{ActorArmorSnapshot, CanonicalItemStack, WorldStream};
use protocol::ActorHandedness;

use super::runtime::{ActorEquipmentInput, HeldKind, WornItem};
use crate::ui_runtime::UiRuntime;

/// A drawable worn item, or `None` for an empty or unresolved stack.
pub(super) fn worn_item(item: &CanonicalItemStack, dye_rgb: Option<u32>) -> Option<WornItem> {
    if item.identity.is_empty() {
        return None;
    }
    Some(WornItem {
        identifier: item.identifier.clone()?,
        metadata: item.identity.metadata,
        kind: match item.visual {
            ItemVisualRoute::Compiled(_) => HeldKind::Sprite,
            ItemVisualRoute::BlockItem(visual) => HeldKind::Block(visual.0),
            _ => HeldKind::Other,
        },
        dye_rgb,
    })
}

fn armor_slots(armor: Option<&ActorArmorSnapshot>) -> [Option<WornItem>; 4] {
    let Some(armor) = armor else {
        return [None, None, None, None];
    };
    [
        &armor.helmet,
        &armor.chestplate,
        &armor.leggings,
        &armor.boots,
    ]
    .map(|piece| worn_item(&piece.item, piece.dye_rgb))
}

/// A remote actor's equipment from its replicated equipment and armor events.
pub(crate) fn remote_input(stream: &WorldStream, runtime_id: u64) -> ActorEquipmentInput {
    let held = |hand| {
        stream
            .actor_equipment_in_hand(runtime_id, hand)
            .and_then(|equipment| worn_item(&equipment.item, None))
    };
    let actor = stream.actor(runtime_id);
    ActorEquipmentInput {
        main: held(ActorHandedness::Right),
        off: held(ActorHandedness::Left),
        armor: armor_slots(stream.actor_armor(runtime_id)),
        sneaking: actor.is_some_and(|actor| actor.is_sneaking()),
        sleeping: actor.is_some_and(|actor| actor.is_sleeping()),
    }
}

/// The local player's equipment: held stacks from the client-owned inventory, armor from the
/// authoritative armor event.
pub(crate) fn local_input(
    stream: &WorldStream,
    ui: Option<&UiRuntime>,
    runtime_id: u64,
) -> ActorEquipmentInput {
    let actor = stream.actor(runtime_id);
    let resolve = |stack: &protocol::NetworkItemStack| {
        stream
            .canonical_item_stack(stack)
            .and_then(|item| worn_item(&item, None))
    };
    ActorEquipmentInput {
        main: ui.and_then(UiRuntime::selected_stack).and_then(resolve),
        off: ui
            .and_then(|ui| ui.gameplay_hud().offhand_stack())
            .and_then(resolve),
        armor: armor_slots(stream.actor_armor(runtime_id)),
        sneaking: actor.is_some_and(|actor| actor.is_sneaking()),
        sleeping: actor.is_some_and(|actor| actor.is_sleeping()),
    }
}
