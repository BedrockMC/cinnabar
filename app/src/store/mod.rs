//! Marketplace client state behind the vanilla store screens: the control worker, the purchase flow,
//! the store data sources for the JSON-UI engine and the bounded offer-image cache.
#![allow(dead_code, reason = "consumed by the store screens as they land")]

mod bindings;
mod flow;
mod images;
mod state;
mod worker;

pub(crate) use bindings::{balance_text, offer_items, offer_list_source};
pub(crate) use flow::{PurchaseDialog, PurchaseFlow, new_purchase_id};
pub(crate) use images::TextureCache;
pub(crate) use state::StoreState;
pub(crate) use worker::{StoreError, StoreEvent, StoreRequest, StoreWorker};
