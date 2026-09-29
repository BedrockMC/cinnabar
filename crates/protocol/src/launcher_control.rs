//! Launcher control client (realms, friends, connect, account, events), re-exported so the app
//! reaches the bridge through this facade.

pub use bridge::{
    Account, AuthState, BridgeError, ConnectTarget, Events, Friend, Realm, ServerDisconnect,
    TransferPending, account_status, connect_target, list_friends, list_realms, poll_events,
    sign_out,
};
