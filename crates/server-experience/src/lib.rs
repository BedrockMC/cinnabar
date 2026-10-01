//! Opt-in Cinnabar extensions. Nothing here changes vanilla login or packet IDs.

pub mod bundle;
pub mod cache;
pub mod crypto;
pub mod fetch;
pub mod manifest;
pub mod negotiation;
pub mod policy;
pub mod session;
pub mod trust;
pub mod wire;

#[cfg(test)]
mod tests;
