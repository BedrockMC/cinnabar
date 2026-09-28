use valentine::bedrock::version::v1_26_44::{
    ActorEventPacket, EnumsActorEvent, TakeItemActorPacket,
};

use crate::ActorEvent;

/// Server-announced actor events the client visualises; ids with no client-side visual are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorStatusKind {
    Hurt,
    Death,
    /// Failed taming: smoke particles.
    TamingFailed,
    /// Successful taming: heart particles.
    TamingSucceeded,
    ShakeWetness,
    EatGrass,
    LoveHearts,
    VillagerAngry,
    VillagerHappy,
    WitchHatMagic,
    FireworksExplode,
    DrinkPotion,
    ThrowPotion,
    PrimeTntMinecart,
    PrimeCreeper,
    TotemActivate,
    SpawnAlive,
    LeashDestroyed,
    ZombieConverting,
    Puke,
    DrinkMilk,
    Feed,
    ActorGrowUp,
}

/// One actor status event, addressed by runtime id (the wire carries no dimension).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorStatusEvent {
    pub runtime_id: u64,
    pub kind: ActorStatusKind,
    /// Event-specific payload; its meaning depends on `kind` and is unused for most kinds.
    pub data: i32,
}

/// A dropped item was picked up: the item flies to `collector_runtime_id` and is then removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorTakeItemEvent {
    pub item_runtime_id: u64,
    pub collector_runtime_id: u64,
}

pub(crate) fn normalize_take_item_actor(packet: TakeItemActorPacket) -> ActorEvent {
    ActorEvent::TakeItem(ActorTakeItemEvent {
        item_runtime_id: packet.item_runtime_id.actor_runtime_id,
        collector_runtime_id: packet.actor_runtime_id.actor_runtime_id,
    })
}

/// Maps an ActorEvent packet to a status event, or `None` for ids the client draws nothing for.
pub(crate) fn normalize_actor_event(packet: ActorEventPacket) -> Option<ActorEvent> {
    let kind = match packet.event_id {
        EnumsActorEvent::Hurt | EnumsActorEvent::HurtWithoutReceivingDamage => {
            ActorStatusKind::Hurt
        }
        EnumsActorEvent::Death | EnumsActorEvent::InstantDeath => ActorStatusKind::Death,
        EnumsActorEvent::TamingFailed => ActorStatusKind::TamingFailed,
        EnumsActorEvent::TamingSucceeded => ActorStatusKind::TamingSucceeded,
        EnumsActorEvent::ShakeWetness => ActorStatusKind::ShakeWetness,
        EnumsActorEvent::EatGrass => ActorStatusKind::EatGrass,
        EnumsActorEvent::LoveHearts | EnumsActorEvent::InLoveHearts => ActorStatusKind::LoveHearts,
        EnumsActorEvent::VillagerAngry => ActorStatusKind::VillagerAngry,
        EnumsActorEvent::VillagerHappy => ActorStatusKind::VillagerHappy,
        EnumsActorEvent::WitchHatMagic => ActorStatusKind::WitchHatMagic,
        EnumsActorEvent::FireworksExplode => ActorStatusKind::FireworksExplode,
        EnumsActorEvent::DrinkPotion => ActorStatusKind::DrinkPotion,
        EnumsActorEvent::ThrowPotion => ActorStatusKind::ThrowPotion,
        EnumsActorEvent::PrimeTntcart => ActorStatusKind::PrimeTntMinecart,
        EnumsActorEvent::PrimeCreeper => ActorStatusKind::PrimeCreeper,
        EnumsActorEvent::TalismanActivate => ActorStatusKind::TotemActivate,
        EnumsActorEvent::SpawnAlive => ActorStatusKind::SpawnAlive,
        EnumsActorEvent::LeashDestroyed => ActorStatusKind::LeashDestroyed,
        EnumsActorEvent::ZombieConverting => ActorStatusKind::ZombieConverting,
        EnumsActorEvent::Puke => ActorStatusKind::Puke,
        EnumsActorEvent::DrinkMilk => ActorStatusKind::DrinkMilk,
        EnumsActorEvent::Feed => ActorStatusKind::Feed,
        EnumsActorEvent::ActorGrowUp => ActorStatusKind::ActorGrowUp,
        _ => return None,
    };
    Some(ActorEvent::Status(ActorStatusEvent {
        runtime_id: packet.target_runtime_id.actor_runtime_id,
        kind,
        data: packet.data,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(event_id: EnumsActorEvent) -> ActorEventPacket {
        let mut packet = ActorEventPacket::default();
        packet.target_runtime_id.actor_runtime_id = 9;
        packet.event_id = event_id;
        packet.data = 4;
        packet
    }

    #[test]
    fn hurt_and_death_ids_normalize_by_runtime_id() {
        assert_eq!(
            normalize_actor_event(packet(EnumsActorEvent::Hurt)),
            Some(ActorEvent::Status(ActorStatusEvent {
                runtime_id: 9,
                kind: ActorStatusKind::Hurt,
                data: 4,
            }))
        );
        assert!(matches!(
            normalize_actor_event(packet(EnumsActorEvent::InstantDeath)),
            Some(ActorEvent::Status(ActorStatusEvent {
                kind: ActorStatusKind::Death,
                ..
            }))
        ));
    }

    #[test]
    fn unknown_and_invisible_ids_are_skipped() {
        assert_eq!(
            normalize_actor_event(packet(EnumsActorEvent::Unknown(200))),
            None
        );
        assert_eq!(normalize_actor_event(packet(EnumsActorEvent::Jump)), None);
    }

    #[test]
    fn add_item_actor_spawns_a_fixed_identifier_item_entity() {
        let mut packet = valentine::bedrock::version::v1_26_44::AddItemActorPacket::default();
        packet.target_actor_id.actor_unique_id = -5;
        packet.target_runtime_id.actor_runtime_id = 12;
        packet.position.y = 64.0;
        let Ok(ActorEvent::Spawn(spawn)) = super::super::normalize_add_item_actor(packet, 0) else {
            panic!("item actor spawn");
        };
        assert_eq!((spawn.unique_id, spawn.runtime_id), (-5, 12));
        assert!(matches!(
            &spawn.kind,
            crate::ActorKind::Entity { identifier } if identifier.as_ref() == "minecraft:item"
        ));
        assert_eq!(spawn.position[1], 64.0);
    }

    #[test]
    fn take_item_names_item_and_collector() {
        let mut packet = TakeItemActorPacket::default();
        packet.item_runtime_id.actor_runtime_id = 3;
        packet.actor_runtime_id.actor_runtime_id = 4;
        assert_eq!(
            normalize_take_item_actor(packet),
            ActorEvent::TakeItem(ActorTakeItemEvent {
                item_runtime_id: 3,
                collector_runtime_id: 4,
            })
        );
    }
}
