//! Marketplace client state behind the vanilla store screens: the control worker, the purchase flow,
//! the store data sources for the JSON-UI engine and the bounded offer-image cache.

mod action;
mod bindings;
mod driver;
mod flow;
mod images;
mod screens;
mod settings;
mod snapshot;
mod state;
mod worker;

pub(crate) use action::StoreAction;
pub(crate) use driver::drive_store;
pub(crate) use screens::{ScreenSpec, StoreScreens, screens};
pub(crate) use settings::SETTINGS_FILE;
pub(crate) use snapshot::StoreSnapshot;
pub(crate) use state::StoreState;
pub(crate) use worker::StoreWorker;

use json_ui::HitRegion;

/// The store action a pressed `region` means while `snapshot` is showing.
pub(crate) fn action(snapshot: Option<&StoreSnapshot>, region: &HitRegion) -> Option<StoreAction> {
    let snapshot = snapshot?;
    action::action_for(snapshot.view, snapshot.modal_active(), region)
}

/// The start-screen Marketplace button.
pub(crate) const OPEN: StoreAction = StoreAction::Open;
