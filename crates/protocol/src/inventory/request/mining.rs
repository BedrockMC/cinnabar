use thiserror::Error;
use valentine::bedrock::version::v1_26_44::{
    EnumsItemStackRequestActionType, EnumsTextProcessingEventOrigin,
    ItemStackRequestCerealMineBlockActionData, ItemStackRequestCerealRequestData,
    ItemStackRequestCerealRequestDataActionsItem,
    TypedClientNetIdstructItemStackRequestIdTagint32T0,
};

/// A bounded mining request carrier, not proof of current inventory authority.
/// Callers must independently establish item applicability and current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MineBlockRequest {
    request_id: i32,
    hotbar_slot: u8,
    predicted_durability: i32,
    stack_network_id: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MineBlockRequestError {
    #[error("mining request ID must be odd and at most -3")]
    InvalidRequestId,
    #[error("mining hotbar slot must be within 0..=8")]
    InvalidHotbarSlot,
    #[error("predicted mining durability must be nonnegative")]
    InvalidPredictedDurability,
    #[error("mining stack network ID must be positive")]
    InvalidStackNetworkId,
}

impl MineBlockRequest {
    #[must_use]
    pub const fn request_id(&self) -> i32 {
        self.request_id
    }

    pub fn new(
        request_id: i32,
        hotbar_slot: u8,
        predicted_durability: i32,
        stack_network_id: i32,
    ) -> Result<Self, MineBlockRequestError> {
        if request_id > -3 || request_id & 1 == 0 {
            return Err(MineBlockRequestError::InvalidRequestId);
        }
        if hotbar_slot > 8 {
            return Err(MineBlockRequestError::InvalidHotbarSlot);
        }
        if predicted_durability < 0 {
            return Err(MineBlockRequestError::InvalidPredictedDurability);
        }
        if stack_network_id <= 0 {
            return Err(MineBlockRequestError::InvalidStackNetworkId);
        }
        Ok(Self {
            request_id,
            hotbar_slot,
            predicted_durability,
            stack_network_id,
        })
    }

    pub(crate) fn packed(self) -> ItemStackRequestCerealRequestData {
        ItemStackRequestCerealRequestData {
            client_request_id: TypedClientNetIdstructItemStackRequestIdTagint32T0 {
                id: self.request_id,
            },
            actions: vec![
                ItemStackRequestCerealRequestDataActionsItem::MineBlockActionData(Box::new(
                    ItemStackRequestCerealMineBlockActionData {
                        actiontype: EnumsItemStackRequestActionType::ScreenHudMineBlock,
                        slot: i32::from(self.hotbar_slot),
                        predicted_durability: self.predicted_durability,
                        net_id_variant: self.stack_network_id,
                    },
                )),
            ],
            strings_to_filter: Vec::new(),
            strings_to_filter_origin: EnumsTextProcessingEventOrigin::Unknown,
        }
    }
}
