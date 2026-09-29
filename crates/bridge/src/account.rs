use std::path::Path;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::endpoint::EndpointKind;
use crate::status::{CONTROL_MAX_FRAME_LEN, RpcError, TransferPending, invalid};
use crate::{BridgeError, FramedStream};

const REQUEST_ID: u64 = 1;
const SCHEMA_VERSION: u32 = 1;

/// One Realm the account can join; `target` is its stable join id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Realm {
    pub name: String,
    pub state: String,
    pub target: String,
    #[serde(default)]
    pub address: Option<String>,
}

/// One friend's joinable world; `xuid` identifies it for [`ConnectTarget::Friend`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Friend {
    pub gamertag: String,
    pub xuid: String,
    pub world_name: String,
    pub members: u32,
    pub max_members: u32,
    #[serde(default)]
    pub handle_id: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}

/// Where the next client connection goes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectTarget {
    /// A `host:port` server.
    RakNet(String),
    /// A Realm id from [`Realm::target`] (`realm_id/<id>` → `<id>`).
    Realm(String),
    /// A friend's XUID from [`Friend::xuid`].
    Friend(String),
}

impl ConnectTarget {
    fn params(&self) -> ConnectParams<'_> {
        let (kind, value) = match self {
            Self::RakNet(value) => ("raknet", value),
            Self::Realm(value) => ("realm", value),
            Self::Friend(value) => ("friend", value),
        };
        ConnectParams { kind, value }
    }
}

#[derive(Serialize)]
struct ConnectParams<'a> {
    kind: &'static str,
    value: &'a str,
}

/// Sign-in state of the core.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// The core runs without a Microsoft account.
    Offline,
    SignedOut,
    /// Show `verification_uri` and `user_code` to the player.
    AwaitingCode,
    SignedIn,
    Failed,
}

/// Sign-in state plus the fields that belong to it; `reason` never carries secrets or paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Account {
    pub state: AuthState,
    #[serde(default)]
    pub verification_uri: Option<String>,
    #[serde(default)]
    pub user_code: Option<String>,
    #[serde(default)]
    pub gamertag: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// The server's own reason for ending a session; `sequence` grows with every disconnect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ServerDisconnect {
    pub reason: i32,
    pub message: String,
    pub sequence: u64,
}

/// Pollable core events: auth state plus the newest disconnect and pending transfer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Events {
    pub auth: Account,
    #[serde(default)]
    pub disconnect: Option<ServerDisconnect>,
    #[serde(default)]
    pub transfer: Option<TransferPending>,
}

#[derive(Serialize)]
struct Request<'a, P: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<P>,
}

#[derive(Deserialize)]
struct Envelope<R> {
    jsonrpc: String,
    id: u64,
    #[serde(default = "none")]
    result: Option<Versioned<R>>,
    #[serde(default)]
    error: Option<RpcError>,
}

fn none<T>() -> Option<T> {
    None
}

#[derive(Deserialize)]
struct Versioned<R> {
    schema_version: u32,
    #[serde(flatten)]
    body: R,
}

#[derive(Deserialize)]
struct Empty {}

#[derive(Deserialize)]
struct RealmsBody {
    realms: Vec<Realm>,
}

#[derive(Deserialize)]
struct FriendsBody {
    friends: Vec<Friend>,
}

#[derive(Deserialize)]
struct AccountBody {
    account: Account,
}

async fn call<R: DeserializeOwned, P: Serialize>(
    socket_dir: &Path,
    method: &str,
    params: Option<P>,
) -> Result<R, BridgeError> {
    let stream = crate::endpoint::connect(socket_dir, EndpointKind::Control).await?;
    let mut framed = FramedStream::with_max(stream, CONTROL_MAX_FRAME_LEN);
    let request = serde_json::to_vec(&Request {
        jsonrpc: "2.0",
        id: REQUEST_ID,
        method,
        params,
    })?;
    framed.send(Bytes::from(request)).await?;
    let response = framed.next().await.ok_or(BridgeError::ControlClosed)??;
    parse_response(&response)
}

fn parse_response<R: DeserializeOwned>(payload: &[u8]) -> Result<R, BridgeError> {
    let response: Envelope<R> = serde_json::from_slice(payload)?;
    if response.jsonrpc != "2.0" {
        return invalid("jsonrpc must be exactly 2.0");
    }
    if response.id != REQUEST_ID {
        return invalid("response id does not match the request");
    }
    match (response.result, response.error) {
        (Some(result), None) if result.schema_version == SCHEMA_VERSION => Ok(result.body),
        (Some(_), None) => invalid("unsupported launcher schema version"),
        (None, Some(error)) => Err(BridgeError::ControlRpc {
            code: error.code,
            message: error.message,
        }),
        (Some(_), Some(_)) => invalid("response contains both result and error"),
        (None, None) => invalid("response contains neither result nor error"),
    }
}

/// Lists the account's Realms.
pub async fn list_realms(socket_dir: &Path) -> Result<Vec<Realm>, BridgeError> {
    let body: RealmsBody = call::<_, ()>(socket_dir, "realms_list.v1", None).await?;
    Ok(body.realms)
}

/// Lists friends' joinable worlds.
pub async fn list_friends(socket_dir: &Path) -> Result<Vec<Friend>, BridgeError> {
    let body: FriendsBody = call::<_, ()>(socket_dir, "friends_list.v1", None).await?;
    Ok(body.friends)
}

/// Selects the upstream for the next game-socket connection.
pub async fn connect_target(socket_dir: &Path, target: &ConnectTarget) -> Result<(), BridgeError> {
    call::<Empty, _>(socket_dir, "connect.v1", Some(target.params())).await?;
    Ok(())
}

/// Reads the sign-in state.
pub async fn account_status(socket_dir: &Path) -> Result<Account, BridgeError> {
    let body: AccountBody = call::<_, ()>(socket_dir, "account_status.v1", None).await?;
    Ok(body.account)
}

/// Deletes the cached Microsoft tokens and returns the resulting state.
pub async fn sign_out(socket_dir: &Path) -> Result<Account, BridgeError> {
    let body: AccountBody = call::<_, ()>(socket_dir, "sign_out.v1", None).await?;
    Ok(body.account)
}

/// Reads the auth state and the newest disconnect and transfer.
pub async fn poll_events(socket_dir: &Path) -> Result<Events, BridgeError> {
    call::<Events, ()>(socket_dir, "events.v1", None).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_params_match_the_wire_contract() {
        let encoded = serde_json::to_string(&Request {
            jsonrpc: "2.0",
            id: 1,
            method: "connect.v1",
            params: Some(ConnectTarget::Realm("42".into()).params()),
        })
        .expect("encode");
        assert_eq!(
            encoded,
            r#"{"jsonrpc":"2.0","id":1,"method":"connect.v1","params":{"kind":"realm","value":"42"}}"#
        );
        let raknet = serde_json::to_string(&ConnectTarget::RakNet("a:1".into()).params());
        assert_eq!(
            raknet.expect("encode"),
            r#"{"kind":"raknet","value":"a:1"}"#
        );
        let friend = serde_json::to_string(&ConnectTarget::Friend("9".into()).params());
        assert_eq!(friend.expect("encode"), r#"{"kind":"friend","value":"9"}"#);
    }

    #[test]
    fn parses_realm_and_friend_lists() {
        let realms = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,"realms":[
            {"name":"R","state":"OPEN","target":"realm_id/7","address":"1.2.3.4:19132"}]}}"#;
        let body: RealmsBody = parse_response(realms).expect("realms");
        assert_eq!(body.realms[0].target, "realm_id/7");
        let friends = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,"friends":[
            {"gamertag":"F","xuid":"123","world_name":"W","members":1,"max_members":8}]}}"#;
        let body: FriendsBody = parse_response(friends).expect("friends");
        assert_eq!(body.friends[0].xuid, "123");
        assert_eq!(body.friends[0].handle_id, None);
        let empty = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,"realms":[]}}"#;
        assert!(
            parse_response::<RealmsBody>(empty)
                .expect("empty")
                .realms
                .is_empty()
        );
    }

    #[test]
    fn parses_account_and_events() {
        let account = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,
            "account":{"state":"awaiting_code","verification_uri":"https://x.test/l","user_code":"AB12"}}}"#;
        let body: AccountBody = parse_response(account).expect("account");
        assert_eq!(body.account.state, AuthState::AwaitingCode);
        assert_eq!(body.account.user_code.as_deref(), Some("AB12"));
        let events = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,
            "auth":{"state":"signed_in","gamertag":"Steve"},
            "disconnect":{"reason":7,"message":"banned","sequence":2},
            "transfer":{"host":"n.example","port":19133,"sequence":1}}}"#;
        let events: Events = parse_response(events).expect("events");
        assert_eq!(events.auth.gamertag.as_deref(), Some("Steve"));
        assert_eq!(events.disconnect.expect("disconnect").sequence, 2);
        assert_eq!(events.transfer.expect("transfer").port, 19133);
        let quiet =
            br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,"auth":{"state":"offline"}}}"#;
        let quiet: Events = parse_response(quiet).expect("quiet");
        assert_eq!(quiet.auth.state, AuthState::Offline);
        assert!(quiet.disconnect.is_none() && quiet.transfer.is_none());
    }

    #[test]
    fn surfaces_rpc_errors_and_rejects_bad_envelopes() {
        let error =
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32020,"message":"Not signed in"}}"#;
        assert!(matches!(
            parse_response::<Empty>(error),
            Err(BridgeError::ControlRpc { code: -32020, .. })
        ));
        let wrong_schema = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":2}}"#;
        assert!(parse_response::<Empty>(wrong_schema).is_err());
        let wrong_id = br#"{"jsonrpc":"2.0","id":9,"result":{"schema_version":1}}"#;
        assert!(parse_response::<Empty>(wrong_id).is_err());
        let neither = br#"{"jsonrpc":"2.0","id":1}"#;
        assert!(parse_response::<Empty>(neither).is_err());
    }
}
