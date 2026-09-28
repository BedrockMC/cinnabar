//! The one canonical container-address projection.
//!
//! Bedrock names an inventory cell three different ways on the wire: a legacy
//! bare window id plus slot index (`InventoryContent`/`InventorySlot`), a full
//! container name plus dynamic id alongside that window id, and a response
//! container that carries only the container name plus dynamic id. Matching
//! any one wire field alone collapses distinct surfaces onto each other — a
//! cursor update can land in hotbar cell 0, or an offhand rewrite can clear
//! unseen inventory cells.
//!
//! [`project_container_cell`] is the single resolver every admission, lookup,
//! and accepted-response path routes through. It maps the wire triple (window
//! id, decoded container-name code, slot index) onto one explicit
//! [`CanonicalCell`], so a Content event, a Slot event, and an accepted item
//! stack response describing the same physical cell all resolve to the same
//! canonical value while distinct cells stay distinct.
//!
//! Only identities today's protocol layer actually decodes are enumerated;
//! anything else — including decoded container names this client has no
//! reviewed mapping for — resolves to `None`, and callers treat that as odd
//! but well-formed data: a typed counted skip, never a mutation and never a
//! disconnect. One shape is routed by prior admission rather than a reviewed
//! live mapping: the `InventoryContainer` alias on legacy window 0 (see
//! [`project_container_cell`]), which the repository's pinned fixture corpus
//! encodes and base admission applied to player cells before this projection
//! existed.

use super::ContainerIdentity;
use super::container_policy::{CONTAINER_NAME_CREATED_OUTPUT, CONTAINER_NAME_HOTBAR};

/// `EnumsContainerEnumName::ArmorContainer`, the player armor surface.
pub const CONTAINER_NAME_ARMOR: u8 = 6;
/// `EnumsContainerEnumName::LevelEntityContainer`, the generic screen-specific
/// storage surface keyed by its dynamic container id.
pub const CONTAINER_NAME_LEVEL_ENTITY: u8 = 7;
/// `EnumsContainerEnumName::CombinedHotbarAndInventoryContainer`, the combined
/// player inventory surface every gesture request names.
pub const CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY: u8 = 12;
/// The distinct personal crafting input surface; wire cells 28..31.
pub const CONTAINER_NAME_CRAFT_INPUT: u8 = 13;
/// `EnumsContainerEnumName::InventoryContainer`, the named player-inventory
/// alias live servers send riding the legacy window id (the pinned
/// gophertunnel fixture corpus encodes exactly this shape).
pub const CONTAINER_NAME_INVENTORY: u8 = 29;
/// `EnumsContainerEnumName::OffhandContainer`.
pub const CONTAINER_NAME_OFFHAND: u8 = 34;
/// `EnumsContainerEnumName::CursorContainer`.
pub const CONTAINER_NAME_CURSOR: u8 = 59;

/// The combined player-inventory window id (`CONTAINER_ID_INVENTORY`).
pub const PLAYER_INVENTORY_WINDOW_ID: i32 = 0;
/// The legacy offhand window id (`CONTAINER_ID_OFFHAND`), which servers send
/// without a full container name.
pub const OFFHAND_WINDOW_ID: i32 = 119;
/// The legacy armor window id, addressed like the offhand window.
pub const ARMOR_WINDOW_ID: i32 = 120;

/// One canonical inventory cell in the explicit cross-surface address space.
///
/// Distinct members never collide: the thirty-six
/// [`CanonicalCell::PlayerInventory`] indices cover the nine hotbar cells
/// (0..9) followed by the twenty-seven main-inventory cells (9..36), and
/// armor, offhand, cursor, and the dynamic generic-storage surface are
/// separate values regardless of what legacy window id happened to ride the
/// packet.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CanonicalCell {
    /// One combined player-inventory cell: indices 0..9 are the hotbar cells,
    /// 9..36 the main-inventory cells.
    PlayerInventory(u8),
    /// One armor-surface cell addressed by its container-relative index.
    Armor(u8),
    /// The single offhand cell.
    Offhand,
    /// The single held-stack cursor cell.
    Cursor,
    /// One of four personal crafting cells, never a player-inventory alias.
    CraftInput(u8),
    /// One of nine crafting-table cells (wire slots 32..=40).
    TableCraftInput(u8),
    /// The created-output cell (wire slot 50).
    CreatedOutput,
    /// One screen-specific generic storage cell identified by its open
    /// container's dynamic id.
    GenericStorage { dynamic_id: Option<u32>, slot: u16 },
}

impl CanonicalCell {
    /// Whether this cell belongs to the combined player-inventory surface.
    #[cfg(test)]
    #[must_use]
    pub const fn is_player_inventory(self) -> bool {
        matches!(self, Self::PlayerInventory(_))
    }
}

/// Resolves one wire cell address onto its canonical cell, or `None` when the
/// container identity does not route onto the canonical space.
///
/// Named containers resolve by their decoded container-name code alone, so a
/// Content event, a Slot event, and an accepted item stack response (whose
/// container carries no window id) converge on the same value. One reviewed
/// prior-admission exception: the `InventoryContainer` alias addresses the
/// player inventory only alongside legacy window 0 — exactly how base
/// admission treated it before this projection existed (the pinned fixture
/// corpus writes `window_id 0 + InventoryContainer + slot`), so that shape
/// keeps routing onto player cells while any further named aliases wait for
/// live adjudication. Unnamed addresses fall back to the two legacy window
/// ids the client recognizes: window 0 as the combined player inventory and
/// window 119 as the offhand.
///
/// Slot-index sanity is part of the mapping: a cursor or offhand address only
/// exists at index 0, and player-inventory indices outside `0..36` are not
/// player-inventory cells at all.
#[must_use]
pub fn project_container_cell(identity: &ContainerIdentity, slot: u16) -> Option<CanonicalCell> {
    match identity.slot_type {
        Some(CONTAINER_NAME_CRAFT_INPUT) => {
            let index = u8::try_from(slot.checked_sub(28)?).ok()?;
            match index {
                0..4 => Some(CanonicalCell::CraftInput(index)),
                4..13 => Some(CanonicalCell::TableCraftInput(index - 4)),
                _ => None,
            }
        }
        Some(CONTAINER_NAME_CREATED_OUTPUT) => (slot == 50).then_some(CanonicalCell::CreatedOutput),
        Some(CONTAINER_NAME_CURSOR) => (slot == 0).then_some(CanonicalCell::Cursor),
        Some(CONTAINER_NAME_ARMOR) => armor_cell(slot),
        // Requests address the single offhand cell as wire slot 1 and
        // responses echo it; slot updates use index 0.
        Some(CONTAINER_NAME_OFFHAND) => matches!(slot, 0 | 1).then_some(CanonicalCell::Offhand),
        // Vanilla requests name hotbar cells this way; responses echo it.
        Some(CONTAINER_NAME_HOTBAR)
            if identity.dynamic_id.is_none()
                && matches!(identity.window_id, None | Some(PLAYER_INVENTORY_WINDOW_ID)) =>
        {
            (slot < 9).then(|| player_inventory_cell(slot)).flatten()
        }
        Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY)
            if matches!(identity.window_id, None | Some(PLAYER_INVENTORY_WINDOW_ID)) =>
        {
            player_inventory_cell(slot)
        }
        // Servers name player traffic `InventoryContainer` on legacy window
        // 0, and vanilla requests name main-inventory cells this way with
        // responses echoing it without a window.
        Some(CONTAINER_NAME_INVENTORY)
            if identity.dynamic_id.is_none()
                && matches!(identity.window_id, None | Some(PLAYER_INVENTORY_WINDOW_ID)) =>
        {
            player_inventory_cell(slot)
        }
        Some(CONTAINER_NAME_LEVEL_ENTITY) => Some(CanonicalCell::GenericStorage {
            dynamic_id: identity.dynamic_id,
            slot,
        }),
        // Every other decoded container name — crafting inputs, furnaces,
        // trades, unknown codes — has no reviewed canonical mapping here.
        Some(_) => None,
        None => match identity.window_id {
            Some(PLAYER_INVENTORY_WINDOW_ID) => player_inventory_cell(slot),
            Some(OFFHAND_WINDOW_ID) if slot == 0 => Some(CanonicalCell::Offhand),
            Some(ARMOR_WINDOW_ID) => armor_cell(slot),
            _ => None,
        },
    }
}

fn armor_cell(slot: u16) -> Option<CanonicalCell> {
    let slot = u8::try_from(slot).ok()?;
    (slot < super::request::ARMOR_SLOTS).then_some(CanonicalCell::Armor(slot))
}

fn player_inventory_cell(slot: u16) -> Option<CanonicalCell> {
    let slots = u16::from(super::request::PLAYER_INVENTORY_SLOTS);
    if slot < slots {
        #[allow(
            clippy::cast_possible_truncation,
            reason = "guarded by the PLAYER_INVENTORY_SLOTS bound above"
        )]
        Some(CanonicalCell::PlayerInventory(slot as u8))
    } else {
        None
    }
}

/// Whether an identity is the personal UI inventory (cursor, crafting cells,
/// created output) as servers address it.
#[must_use]
pub fn is_personal_ui_inventory(identity: &ContainerIdentity) -> bool {
    is_personal_ui_storage(identity)
}

fn is_personal_ui_storage(identity: &ContainerIdentity) -> bool {
    identity.window_id == Some(124)
        && identity.slot_type == Some(0)
        && identity.dynamic_id.is_none()
}

/// Selects the four personal input references from an exact UI storage snapshot.
/// This contextual observation does not add a canonical or ordinary ledger alias.
#[must_use]
pub fn personal_craft_content_indices(
    identity: &ContainerIdentity,
    slots: usize,
) -> Option<[usize; 4]> {
    (is_personal_ui_storage(identity) && slots == 54).then_some([28, 29, 30, 31])
}

/// Selects one personal input observation from the default UI storage identity.
/// Other UI cells, named containers and cursor authority are not inferred here.
#[must_use]
pub fn personal_craft_slot_index(identity: &ContainerIdentity, slot: u16) -> Option<u8> {
    if !is_personal_ui_storage(identity) {
        return None;
    }
    let index = slot.checked_sub(28)?;
    (index < 4).then_some(u8::try_from(index).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contextual_personal_inputs_require_exact_identity_and_snapshot_shape() {
        let valid = identity(124, Some(0));
        assert_eq!(
            personal_craft_content_indices(&valid, 54),
            Some([28, 29, 30, 31])
        );
        for count in [0, 4, 53, 55, usize::MAX] {
            assert_eq!(personal_craft_content_indices(&valid, count), None);
        }
        for slot in 28..32 {
            assert_eq!(
                personal_craft_slot_index(&valid, slot),
                Some(u8::try_from(slot - 28).unwrap())
            );
            assert_eq!(project_container_cell(&valid, slot), None);
            assert_eq!(
                project_container_cell(&identity(0, None), slot),
                Some(CanonicalCell::PlayerInventory(u8::try_from(slot).unwrap()))
            );
        }
        for slot in [0, 27, 32, 50, 53, u16::MAX] {
            assert_eq!(personal_craft_slot_index(&valid, slot), None);
        }
        for invalid in [
            identity(0, Some(0)),
            identity(123, Some(0)),
            identity(124, None),
            identity(124, Some(CONTAINER_NAME_CRAFT_INPUT)),
            ContainerIdentity {
                window_id: None,
                ..valid
            },
            ContainerIdentity {
                dynamic_id: Some(1),
                ..valid
            },
        ] {
            assert_eq!(personal_craft_content_indices(&invalid, 54), None);
            assert_eq!(personal_craft_slot_index(&invalid, 28), None);
        }
    }

    fn identity(window_id: i32, slot_type: Option<u8>) -> ContainerIdentity {
        ContainerIdentity {
            window_id: Some(window_id),
            slot_type,
            dynamic_id: None,
        }
    }

    /// Encodes one generated container-name variant so the pinned constants
    /// can be checked against the generated enum numbering.
    fn encoded_name(value: valentine::bedrock::version::v1_26_44::EnumsContainerEnumName) -> u8 {
        use valentine::bedrock::codec::BedrockCodec;

        let mut bytes = bytes::BytesMut::with_capacity(1);
        value
            .encode(&mut bytes)
            .expect("a one-byte container name always encodes");
        bytes[0]
    }

    /// Pins the hand-copied container-name constants against the generated
    /// encoder so a valentine renumber fails loudly here instead of silently
    /// misrouting live traffic.
    #[test]
    fn pinned_container_name_constants_match_the_generated_enum_encoding() {
        use valentine::bedrock::version::v1_26_44::EnumsContainerEnumName;

        let pairs = [
            (
                CONTAINER_NAME_CRAFT_INPUT,
                EnumsContainerEnumName::CraftingInputContainer,
            ),
            (CONTAINER_NAME_ARMOR, EnumsContainerEnumName::ArmorContainer),
            (
                CONTAINER_NAME_LEVEL_ENTITY,
                EnumsContainerEnumName::LevelEntityContainer,
            ),
            (
                CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY,
                EnumsContainerEnumName::CombinedHotbarAndInventoryContainer,
            ),
            (
                CONTAINER_NAME_INVENTORY,
                EnumsContainerEnumName::InventoryContainer,
            ),
            (
                CONTAINER_NAME_OFFHAND,
                EnumsContainerEnumName::OffhandContainer,
            ),
            (
                CONTAINER_NAME_CURSOR,
                EnumsContainerEnumName::CursorContainer,
            ),
        ];
        for (pinned, generated) in pairs {
            assert_eq!(pinned, encoded_name(generated));
        }
    }

    #[test]
    fn unnamed_and_named_player_inventory_addresses_converge_on_one_canonical_cell() {
        let expected = CanonicalCell::PlayerInventory(20);
        assert_eq!(
            project_container_cell(&identity(0, None), 20),
            Some(expected)
        );
        assert_eq!(
            project_container_cell(
                &identity(0, Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY)),
                20
            ),
            Some(expected)
        );
        // Accepted responses carry no window id at all.
        assert_eq!(
            project_container_cell(
                &ContainerIdentity {
                    window_id: None,
                    slot_type: Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY),
                    dynamic_id: Some(7),
                },
                20
            ),
            Some(expected)
        );
        // Hotbar and main-inventory ranges are both inside the surface.
        assert_eq!(
            project_container_cell(&identity(0, None), 0),
            Some(CanonicalCell::PlayerInventory(0))
        );
        assert_eq!(
            project_container_cell(&identity(0, None), 35),
            Some(CanonicalCell::PlayerInventory(35))
        );
        // Out-of-range indices are not player-inventory cells.
        assert_eq!(project_container_cell(&identity(0, None), 36), None);
        assert_eq!(
            project_container_cell(
                &identity(0, Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY)),
                4_095
            ),
            None
        );
    }

    #[test]
    fn combined_player_name_requires_the_player_window_when_a_window_is_present() {
        assert_eq!(
            project_container_cell(
                &identity(6, Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY)),
                3,
            ),
            None,
            "inbound Content and Slot traffic cannot relabel another window as player inventory",
        );
        assert_eq!(
            project_container_cell(
                &ContainerIdentity {
                    window_id: None,
                    slot_type: Some(CONTAINER_NAME_COMBINED_HOTBAR_AND_INVENTORY),
                    dynamic_id: Some(7),
                },
                3,
            ),
            Some(CanonicalCell::PlayerInventory(3)),
            "windowless stack-response containers remain routable",
        );
    }

    #[test]
    fn cursor_armor_offhand_and_storage_surfaces_stay_distinct_from_player_cells() {
        assert_eq!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_CURSOR)), 0),
            Some(CanonicalCell::Cursor)
        );
        // A cursor address beyond its single cell does not exist.
        assert_eq!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_CURSOR)), 1),
            None
        );
        assert_eq!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_ARMOR)), 2),
            Some(CanonicalCell::Armor(2))
        );
        assert_ne!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_ARMOR)), 2),
            project_container_cell(&identity(0, None), 2)
        );

        // Both offhand encodings converge; neither touches a player cell.
        assert_eq!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_OFFHAND)), 0),
            Some(CanonicalCell::Offhand)
        );
        assert_eq!(
            project_container_cell(&identity(OFFHAND_WINDOW_ID, None), 0),
            Some(CanonicalCell::Offhand)
        );
        assert_eq!(
            project_container_cell(&identity(OFFHAND_WINDOW_ID, None), 5),
            None
        );

        let storage = project_container_cell(
            &ContainerIdentity {
                window_id: Some(4),
                slot_type: Some(CONTAINER_NAME_LEVEL_ENTITY),
                dynamic_id: Some(9),
            },
            53,
        );
        assert_eq!(
            storage,
            Some(CanonicalCell::GenericStorage {
                dynamic_id: Some(9),
                slot: 53
            })
        );
        assert!(!storage.is_some_and(CanonicalCell::is_player_inventory));
    }

    #[test]
    fn named_inventory_alias_on_the_legacy_window_converges_with_unnamed_window_zero() {
        use valentine::bedrock::version::v1_26_44::EnumsContainerEnumName;

        let inventory_name = encoded_name(EnumsContainerEnumName::InventoryContainer);
        let expected = CanonicalCell::PlayerInventory(4);
        assert_eq!(
            project_container_cell(&identity(0, Some(inventory_name)), 4),
            Some(expected)
        );
        assert_eq!(
            project_container_cell(&identity(0, Some(inventory_name)), 4),
            project_container_cell(&identity(0, None), 4),
        );
        // Responses echo the request's name without a window; other windows
        // and dynamic identities stay unrouted.
        assert_eq!(
            project_container_cell(
                &ContainerIdentity {
                    window_id: None,
                    slot_type: Some(inventory_name),
                    dynamic_id: None,
                },
                13
            ),
            Some(CanonicalCell::PlayerInventory(13))
        );
        assert_eq!(
            project_container_cell(&identity(6, Some(inventory_name)), 4),
            None
        );
        assert_eq!(
            project_container_cell(
                &ContainerIdentity {
                    window_id: None,
                    slot_type: Some(inventory_name),
                    dynamic_id: Some(3),
                },
                4
            ),
            None,
        );
        // Surface bounds still hold.
        assert_eq!(
            project_container_cell(&identity(0, Some(inventory_name)), 36),
            None
        );
    }

    #[test]
    fn unrouted_container_names_and_legacy_windows_resolve_to_none() {
        assert_eq!(project_container_cell(&identity(0, Some(211)), 0), None);
        assert_eq!(
            project_container_cell(&identity(0, Some(CONTAINER_NAME_HOTBAR)), 9),
            None,
            "the hotbar name covers only the nine hotbar cells"
        );
        assert_eq!(
            project_container_cell(&identity(7, Some(CONTAINER_NAME_HOTBAR)), 0),
            None
        );
        assert_eq!(project_container_cell(&identity(-777, None), 0), None);
        assert_eq!(project_container_cell(&identity(119, None), 1), None);
        assert_eq!(project_container_cell(&identity(0, None), 4_096), None);
    }

    #[test]
    fn four_named_crafting_cells_are_distinct_from_player_main_inventory_and_bare_ui() {
        for slot in 28..32 {
            let craft =
                project_container_cell(&identity(124, Some(CONTAINER_NAME_CRAFT_INPUT)), slot);
            assert_eq!(
                craft,
                Some(CanonicalCell::CraftInput(u8::try_from(slot - 28).unwrap()))
            );
            assert_ne!(craft, project_container_cell(&identity(0, None), slot));
            assert_eq!(project_container_cell(&identity(124, None), slot), None);
        }
        assert_eq!(
            project_container_cell(&identity(124, Some(CONTAINER_NAME_CRAFT_INPUT)), 27),
            None
        );
        for slot in 32..41 {
            assert_eq!(
                project_container_cell(&identity(124, Some(CONTAINER_NAME_CRAFT_INPUT)), slot),
                Some(CanonicalCell::TableCraftInput(
                    u8::try_from(slot - 32).unwrap()
                ))
            );
        }
        assert_eq!(
            project_container_cell(&identity(124, Some(CONTAINER_NAME_CRAFT_INPUT)), 41),
            None
        );
    }

    /// Request-shaped names from the owner's captures: hotbar cells, the
    /// offhand's wire slot 1, armor by name or window 120, and created output.
    #[test]
    fn vanilla_request_container_names_resolve_to_their_cells() {
        let window_less = |slot_type| ContainerIdentity {
            window_id: None,
            slot_type: Some(slot_type),
            dynamic_id: None,
        };
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_HOTBAR), 3),
            Some(CanonicalCell::PlayerInventory(3))
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_OFFHAND), 1),
            Some(CanonicalCell::Offhand)
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_OFFHAND), 2),
            None
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_ARMOR), 4),
            Some(CanonicalCell::Armor(4))
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_ARMOR), 5),
            None
        );
        assert_eq!(
            project_container_cell(&identity(ARMOR_WINDOW_ID, None), 2),
            Some(CanonicalCell::Armor(2))
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_CREATED_OUTPUT), 50),
            Some(CanonicalCell::CreatedOutput)
        );
        assert_eq!(
            project_container_cell(&window_less(CONTAINER_NAME_CREATED_OUTPUT), 0),
            None
        );
    }
}
