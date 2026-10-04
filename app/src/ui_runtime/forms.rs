//! Temporary form input and transport adapters; retained state lives in client-ui.
#[cfg(test)]
pub(crate) use client_ui::ui_runtime::forms::FormValue;
pub use client_ui::ui_runtime::forms::{
    EngineFrame, FormTransportError, LocalFormAction, engine_focus, engine_input,
    flush_form_response,
};
mod interaction;
mod network;
pub(crate) use interaction::drive_server_form_input;
pub(crate) use network::flush_server_form_network;
