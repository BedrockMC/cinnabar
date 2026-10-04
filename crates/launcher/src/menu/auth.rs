//! Account status reported to launcher screens.

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthState {
    SignedOut,
    Checking,
    AwaitingCode { uri: String, code: String },
    Authenticated,
    Failed(String),
}
