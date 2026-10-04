//! Store service adapters and launcher state published to client presentation.

mod driver;
mod settings;
mod snapshot;
mod state;
mod worker;

pub(crate) use client_ui::store::{OPEN, SDL_SCREEN, ScreenSpec, StoreScreens, action, screens};
pub(crate) use driver::drive_store;
pub(crate) use launcher::store::StoreAction;
pub(crate) use settings::SETTINGS_FILE;
pub(crate) use snapshot::StoreSnapshot;
pub(crate) use state::StoreState;
pub(crate) use worker::StoreWorker;
