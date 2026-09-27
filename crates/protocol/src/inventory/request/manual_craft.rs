//! Provisional named-recipe manual request; native acceptance remains required.
use crate::{
    InventoryPacketError, ItemRegistryEntry, RecipeCatalog, RecipeHandle, VerifiedNetworkItemStack,
};
use std::sync::Arc;
use valentine::bedrock::version::v1_26_44::{
    EnumsContainerEnumName, EnumsItemStackRequestActionType,
    EnumsItemStackRequestCerealItemDescriptorType, EnumsTextProcessingEventOrigin,
    FullContainerName, ItemStackRequestCerealConsumeActionData,
    ItemStackRequestCerealCraftRecipeActionData, ItemStackRequestCerealCraftResultsActionData,
    ItemStackRequestCerealItemNameDescriptorData,
    ItemStackRequestCerealNetworkItemInstanceDescriptorData,
    ItemStackRequestCerealNetworkItemInstanceDescriptorDataItemDescriptor,
    ItemStackRequestCerealSlotInfoData, ItemStackRequestCerealTakeActionData,
    ItemStackRequestPacket, ItemStackRequestPacketDataRequestData,
    ItemStackRequestPacketDataRequestDataActionsItem,
    TypedClientNetIdstructItemStackRequestIdTagint32T0, TypedServerNetIdstructRecipeNetIdTag,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ManualCraftError {
    #[error("manual recipe context is stale or unavailable")]
    Unavailable,
    #[error("manual recipe inputs or registry binding are unsupported")]
    Unsupported,
    #[error(transparent)]
    Request(#[from] InventoryPacketError),
}

/// A verified current stack in one of the personal 2x2 UI cells, 28..=31.
#[derive(Debug)]
pub struct ManualCraftInput {
    pub slot: u8,
    pub stack: VerifiedNetworkItemStack,
}

#[derive(Debug)]
struct ManualCraftPlan {
    recipe: RecipeHandle,
    output_name: Arc<str>,
    consume: [Option<(u8, u8, i32)>; 4],
}

/// One immutable caller-owned authority snapshot for an atomic request.
/// The ledger consumer must provide current, session-bound, single-flight state;
/// the protocol constructor cannot establish that caller's freshness itself.
#[derive(Debug)]
pub struct ManualCraftSnapshot<'a> {
    pub session: u64,
    pub catalog: &'a RecipeCatalog,
    pub registry: &'a [ItemRegistryEntry],
    pub inputs: [Option<ManualCraftInput>; 4],
    pub cursor: &'a VerifiedNetworkItemStack,
}

fn binding(
    registry: &[ItemRegistryEntry],
    id: i32,
) -> Result<&ItemRegistryEntry, ManualCraftError> {
    let mut matches = registry.iter().filter(|entry| entry.network_id == id);
    let entry = matches.next().ok_or(ManualCraftError::Unsupported)?;
    if matches.next().is_some() {
        return Err(ManualCraftError::Unsupported);
    }
    binding_entry(entry)
}

pub(in crate::inventory) fn binding_entry(
    entry: &ItemRegistryEntry,
) -> Result<&ItemRegistryEntry, ManualCraftError> {
    if (entry.component_based && !entry.canonical_empty_component_data)
        || !super::super::recipes::valid_identifier(&entry.identifier)
    {
        return Err(ManualCraftError::Unsupported);
    }
    Ok(entry)
}

/// Shared value validation. Request cell/stack identities are a separate gate.
pub(in crate::inventory) fn validate_grid<'a>(
    recipe: &super::super::recipes::model::Recipe,
    grid: &[Option<&VerifiedNetworkItemStack>; 4],
    lookup: impl Fn(i32) -> Result<&'a ItemRegistryEntry, ManualCraftError>,
) -> Result<(&'a ItemRegistryEntry, [Option<u8>; 4]), ManualCraftError> {
    let mut counts = [None; 4];
    for (index, input) in grid.iter().enumerate() {
        let cell = index / 2 * usize::from(recipe.width) + index % 2;
        let expected =
            if index % 2 < usize::from(recipe.width) && index / 2 < usize::from(recipe.height) {
                recipe.ingredients[cell].as_ref()
            } else {
                None
            };
        match (expected, input) {
            (None, None) => {}
            (Some(expected), Some(stack)) => {
                if stack.count() < u16::from(expected.count)
                    || stack.metadata() != u32::from(expected.aux)
                    || !super::super::recipes::empty_extra(stack.extra_data())
                    || lookup(stack.network_id())?.identifier.as_ref() != expected.name
                {
                    return Err(ManualCraftError::Unsupported);
                }
                counts[index] = Some(expected.count);
            }
            _ => return Err(ManualCraftError::Unsupported),
        }
    }
    let output = lookup(recipe.output.id)?;
    if output
        .negotiated_max_stack_size
        .is_none_or(|capacity| recipe.output.count > capacity)
    {
        return Err(ManualCraftError::Unsupported);
    }
    Ok((output, counts))
}

impl ManualCraftPlan {
    /// Inputs must be supplied from current server-authoritative cells. No tag
    /// expansion, stack-ID allocator, cursor overwrite or metadata fallback.
    fn prepare(
        catalog: &RecipeCatalog,
        session: u64,
        recipe_id: u32,
        inputs: [Option<ManualCraftInput>; 4],
        registry: &[ItemRegistryEntry],
    ) -> Result<Self, ManualCraftError> {
        if registry.len() > crate::MAX_ITEM_REGISTRY_ENTRIES {
            return Err(ManualCraftError::Unsupported);
        }
        if session == 0 || session != catalog.session() || !catalog.is_available() {
            return Err(ManualCraftError::Unavailable);
        }
        let handle = catalog
            .recipe(recipe_id)
            .ok_or(ManualCraftError::Unavailable)?;
        let grid = std::array::from_fn(|index| inputs[index].as_ref().map(|input| &input.stack));
        let (output, counts) = validate_grid(handle.recipe(), &grid, |id| binding(registry, id))?;
        let mut consume: [Option<(u8, u8, i32)>; 4] = [None; 4];
        for (index, input) in inputs.into_iter().enumerate() {
            if let (Some(count), Some(input)) = (counts[index], input) {
                if input.slot != 28 + index as u8
                    || input.stack.stack_network_id() <= 0
                    || consume
                        .iter()
                        .flatten()
                        .any(|(_, _, id)| *id == input.stack.stack_network_id())
                {
                    return Err(ManualCraftError::Unsupported);
                }
                consume[index] = Some((input.slot, count, input.stack.stack_network_id()));
            }
        }
        Ok(Self {
            output_name: Arc::clone(&output.identifier),
            recipe: handle,
            consume,
        })
    }
}

fn slot(
    container_name: EnumsContainerEnumName,
    slot: u8,
    net_id_variant: i32,
) -> ItemStackRequestCerealSlotInfoData {
    ItemStackRequestCerealSlotInfoData {
        fullcontainername: FullContainerName {
            container_name,
            dynamic_id: None,
        },
        slot,
        net_id_variant,
    }
}

/// The only negative stack reference is this request's newly created output.
/// The caller must have authoritative evidence that the destination cursor is empty.
pub fn manual_craft_packet(
    snapshot: ManualCraftSnapshot<'_>,
    recipe_id: u32,
    request_id: i32,
) -> Result<crate::Packet, ManualCraftError> {
    if request_id >= -1 || request_id & 1 == 0 {
        return Err(InventoryPacketError::InvalidStackRequestId.into());
    }
    let ManualCraftSnapshot {
        session,
        catalog,
        registry,
        inputs,
        cursor,
    } = snapshot;
    let plan = ManualCraftPlan::prepare(catalog, session, recipe_id, inputs, registry)?;
    if cursor.network_id() != 0 || cursor.count() != 0 || !cursor.extra_data().is_empty() {
        return Err(ManualCraftError::Unsupported);
    }
    let output = plan.recipe.recipe().output;
    let mut actions = Vec::with_capacity(7);
    actions.push(
        ItemStackRequestPacketDataRequestDataActionsItem::CraftRecipeActionData(
            ItemStackRequestCerealCraftRecipeActionData {
                actiontype: EnumsItemStackRequestActionType::CraftRecipe,
                recipe_net_id: TypedServerNetIdstructRecipeNetIdTag {
                    raw_id: plan.recipe.network_id(),
                },
                numberofrequestedcrafts: 1,
            },
        ),
    );
    actions.push(ItemStackRequestPacketDataRequestDataActionsItem::CraftResultsActionData(ItemStackRequestCerealCraftResultsActionData {
        actiontype: EnumsItemStackRequestActionType::CraftResults, num_crafts: 1,
        craft_results: vec![ItemStackRequestCerealNetworkItemInstanceDescriptorData {
            item_descriptor: ItemStackRequestCerealNetworkItemInstanceDescriptorDataItemDescriptor::ItemNameDescriptorData(ItemStackRequestCerealItemNameDescriptorData {
                descriptor_type: EnumsItemStackRequestCerealItemDescriptorType::ItemName,
                full_name: plan.output_name.to_string(), aux_value: i32::from(output.aux),
            }), stacksize: u16::from(output.count), block_runtime_id: output.block,
            user_data_buffer: if output.empty_envelope { vec![0;10] } else { Vec::new() },
        }],
    }));
    for (source, count, id) in plan.consume.iter().flatten().copied() {
        actions.push(
            ItemStackRequestPacketDataRequestDataActionsItem::ConsumeActionData(
                ItemStackRequestCerealConsumeActionData {
                    actiontype: EnumsItemStackRequestActionType::Consume,
                    amount: count,
                    source: slot(EnumsContainerEnumName::CraftingInputContainer, source, id),
                },
            ),
        );
    }
    actions.push(
        ItemStackRequestPacketDataRequestDataActionsItem::TakeActionData(Box::new(
            ItemStackRequestCerealTakeActionData {
                actiontype: EnumsItemStackRequestActionType::Take,
                amount: output.count,
                source: slot(
                    EnumsContainerEnumName::CreatedOutputContainer,
                    50,
                    request_id,
                ),
                destination: slot(EnumsContainerEnumName::CursorContainer, 0, 0),
            },
        )),
    );
    Ok(ItemStackRequestPacket {
        requests: vec![ItemStackRequestPacketDataRequestData {
            client_request_id: TypedClientNetIdstructItemStackRequestIdTagint32T0 {
                id: request_id,
            },
            actions,
            strings_to_filter: Vec::new(),
            strings_to_filter_origin: EnumsTextProcessingEventOrigin::Unknown,
        }],
    }
    .into())
}
