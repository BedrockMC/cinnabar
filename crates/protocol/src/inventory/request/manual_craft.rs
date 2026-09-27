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
    if matches.next().is_some()
        || (entry.component_based && !entry.canonical_empty_component_data)
        || !super::super::recipes::valid_identifier(&entry.identifier)
    {
        return Err(ManualCraftError::Unsupported);
    }
    Ok(entry)
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
        let recipe = handle.recipe();
        let output = binding(registry, recipe.output.id)?;
        if output
            .negotiated_max_stack_size
            .is_none_or(|capacity| recipe.output.count > capacity)
        {
            return Err(ManualCraftError::Unsupported);
        }
        let mut consume: [Option<(u8, u8, i32)>; 4] = [None; 4];
        for (index, input) in inputs.into_iter().enumerate() {
            // Recipe rows are compact in the packet; the personal grid has stride 2.
            let cell = index / 2 * usize::from(recipe.width) + index % 2;
            let expected = if index % 2 < usize::from(recipe.width)
                && index / 2 < usize::from(recipe.height)
            {
                recipe.ingredients[cell].as_ref()
            } else {
                None
            };
            match (expected, input) {
                (None, None) => {}
                (Some(expected), Some(input)) => {
                    let stack = &input.stack;
                    if input.slot != 28 + index as u8
                        || stack.stack_network_id() <= 0
                        || stack.count() < u16::from(expected.count)
                        || stack.metadata() != u32::from(expected.aux)
                        || !super::super::recipes::empty_extra(stack.extra_data())
                        || binding(registry, stack.network_id())?.identifier.as_ref()
                            != expected.name
                    {
                        return Err(ManualCraftError::Unsupported);
                    }
                    if consume
                        .iter()
                        .flatten()
                        .any(|(_, _, id)| *id == stack.stack_network_id())
                    {
                        return Err(ManualCraftError::Unsupported);
                    }
                    consume[index] = Some((input.slot, expected.count, stack.stack_network_id()));
                }
                _ => return Err(ManualCraftError::Unsupported),
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
