//! Launcher control client (realms, friends, connect, account, events), re-exported so the app
//! reaches the bridge through this facade.

pub use bridge::{
    Account, Artwork, AuthState, BridgeError, ConnectTarget, Events, FeaturedGame, FeaturedServer,
    Friend, Gathering, Profile, Realm, ServerDisconnect, TransferPending, account_status,
    connect_target, list_featured_servers, list_friends, list_gatherings, list_realms, poll_events,
    profile, sign_out,
};
