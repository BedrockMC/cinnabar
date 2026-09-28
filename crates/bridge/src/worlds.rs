use std::path::Path;

use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};

use crate::endpoint::EndpointKind;
use crate::status::{CONTROL_MAX_FRAME_LEN, RpcError, invalid};
use crate::{BridgeError, FramedStream};

const WORLD_REQUEST_ID: u64 = 1;
const WORLD_SCHEMA_VERSION: u32 = 1;

/// Game mode a world starts players in.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameMode {
    #[default]
    Survival,
    Creative,
    Adventure,
}

/// Terrain generator of a local world.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Generator {
    #[default]
    Normal,
    Flat,
}

/// Difficulty of a local world.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Difficulty {
    Peaceful,
    Easy,
    #[default]
    Normal,
    Hard,
}

/// One saved local world as listed by the core.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct World {
    pub id: String,
    pub name: String,
    pub game_mode: GameMode,
    pub generator: Generator,
    pub difficulty: Difficulty,
    pub seed: i64,
    pub created_unix: i64,
    pub last_played_unix: i64,
}

/// Settings for a new world; a `None` seed is chosen randomly by the core.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct NewWorld {
    pub name: String,
    pub game_mode: GameMode,
    pub generator: Generator,
    pub difficulty: Difficulty,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

/// Lifecycle of the open local world.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorldState {
    Idle,
    Starting,
    Running,
    Stopping,
    Failed,
}

/// Status of the open local world; `error` is a short reason with no local paths.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct WorldStatus {
    pub state: WorldState,
    #[serde(default)]
    pub world_id: Option<String>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Serialize)]
struct WorldRequest<'a, P: Serialize> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<P>,
}

#[derive(Deserialize)]
struct WorldResponse {
    jsonrpc: String,
    id: u64,
    #[serde(default)]
    result: Option<WorldResult>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Default, Deserialize)]
struct WorldResult {
    schema_version: u32,
    #[serde(default)]
    worlds: Vec<World>,
    #[serde(default)]
    world: Option<World>,
    #[serde(default)]
    status: Option<WorldStatus>,
}

#[derive(Serialize)]
struct IdParams<'a> {
    id: &'a str,
}

#[derive(Serialize)]
struct RenameParams<'a> {
    id: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct PauseParams {
    paused: bool,
}

async fn call<P: Serialize>(
    socket_dir: &Path,
    method: &str,
    params: Option<P>,
) -> Result<WorldResult, BridgeError> {
    let stream = crate::endpoint::connect(socket_dir, EndpointKind::Control).await?;
    let mut framed = FramedStream::with_max(stream, CONTROL_MAX_FRAME_LEN);
    let request = serde_json::to_vec(&WorldRequest {
        jsonrpc: "2.0",
        id: WORLD_REQUEST_ID,
        method,
        params,
    })?;
    framed.send(Bytes::from(request)).await?;
    let response = framed.next().await.ok_or(BridgeError::ControlClosed)??;
    parse_world_response(&response)
}

fn parse_world_response(payload: &[u8]) -> Result<WorldResult, BridgeError> {
    let response: WorldResponse = serde_json::from_slice(payload)?;
    if response.jsonrpc != "2.0" {
        return invalid("jsonrpc must be exactly 2.0");
    }
    if response.id != WORLD_REQUEST_ID {
        return invalid("response id does not match the request");
    }
    match (response.result, response.error) {
        (Some(result), None) => {
            if result.schema_version != WORLD_SCHEMA_VERSION {
                return invalid("unsupported world schema version");
            }
            Ok(result)
        }
        (None, Some(error)) => Err(BridgeError::ControlRpc {
            code: error.code,
            message: error.message,
        }),
        (Some(_), Some(_)) => invalid("response contains both result and error"),
        (None, None) => invalid("response contains neither result nor error"),
    }
}

fn require_world(result: WorldResult) -> Result<World, BridgeError> {
    result
        .world
        .map_or_else(|| invalid("response is missing the world"), Ok)
}

fn require_status(result: WorldResult) -> Result<WorldStatus, BridgeError> {
    result
        .status
        .map_or_else(|| invalid("response is missing the world status"), Ok)
}

/// Lists saved worlds, most recently played first.
pub async fn list_worlds(socket_dir: &Path) -> Result<Vec<World>, BridgeError> {
    Ok(call::<()>(socket_dir, "world_list.v1", None).await?.worlds)
}

/// Creates a world and returns its saved metadata.
pub async fn create_world(socket_dir: &Path, world: &NewWorld) -> Result<World, BridgeError> {
    require_world(call(socket_dir, "world_create.v1", Some(world)).await?)
}

/// Renames a world.
pub async fn rename_world(socket_dir: &Path, id: &str, name: &str) -> Result<World, BridgeError> {
    require_world(
        call(
            socket_dir,
            "world_rename.v1",
            Some(RenameParams { id, name }),
        )
        .await?,
    )
}

/// Deletes a world that is not open.
pub async fn delete_world(socket_dir: &Path, id: &str) -> Result<(), BridgeError> {
    call(socket_dir, "world_delete.v1", Some(IdParams { id })).await?;
    Ok(())
}

/// Starts opening a world; poll [`world_status`] until it is running, then connect the game socket.
pub async fn open_world(socket_dir: &Path, id: &str) -> Result<WorldStatus, BridgeError> {
    require_status(call(socket_dir, "world_open.v1", Some(IdParams { id })).await?)
}

/// Saves and stops the open world, or clears a failed open.
pub async fn close_world(socket_dir: &Path) -> Result<WorldStatus, BridgeError> {
    require_status(call::<()>(socket_dir, "world_close.v1", None).await?)
}

/// Freezes or resumes the open world (window focus loss and regain).
pub async fn set_world_paused(socket_dir: &Path, paused: bool) -> Result<WorldStatus, BridgeError> {
    require_status(call(socket_dir, "world_pause.v1", Some(PauseParams { paused })).await?)
}

/// Reads the open world's lifecycle.
pub async fn world_status(socket_dir: &Path) -> Result<WorldStatus, BridgeError> {
    require_status(call::<()>(socket_dir, "world_status.v1", None).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_match_the_wire_contract() {
        let params = serde_json::to_string(&WorldRequest {
            jsonrpc: "2.0",
            id: 1,
            method: "world_open.v1",
            params: Some(IdParams { id: "ab" }),
        })
        .expect("encode");
        assert_eq!(
            params,
            r#"{"jsonrpc":"2.0","id":1,"method":"world_open.v1","params":{"id":"ab"}}"#
        );
        let bare = serde_json::to_string(&WorldRequest::<()> {
            jsonrpc: "2.0",
            id: 1,
            method: "world_list.v1",
            params: None,
        })
        .expect("encode");
        assert_eq!(bare, r#"{"jsonrpc":"2.0","id":1,"method":"world_list.v1"}"#);
    }

    #[test]
    fn new_world_omits_random_seed_and_keeps_zero() {
        let random = serde_json::to_string(&NewWorld {
            name: "a".into(),
            ..NewWorld::default()
        })
        .expect("encode");
        assert_eq!(
            random,
            r#"{"name":"a","game_mode":"survival","generator":"normal","difficulty":"normal"}"#
        );
        let zero = serde_json::to_string(&NewWorld {
            name: "a".into(),
            seed: Some(0),
            ..NewWorld::default()
        })
        .expect("encode");
        assert!(zero.ends_with(r#""seed":0}"#));
    }

    #[test]
    fn parses_list_and_status_results() {
        let list = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,"worlds":[
            {"id":"0123456789abcdef","name":"One","game_mode":"creative","generator":"flat",
             "difficulty":"hard","seed":-5,"created_unix":10,"last_played_unix":20}]}}"#;
        let worlds = parse_world_response(list).expect("list").worlds;
        assert_eq!(worlds.len(), 1);
        assert_eq!(worlds[0].game_mode, GameMode::Creative);
        assert_eq!(worlds[0].seed, -5);
        let status = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":1,
            "status":{"state":"starting","world_id":"x","paused":false}}}"#;
        let status = require_status(parse_world_response(status).expect("status")).expect("status");
        assert_eq!(status.state, WorldState::Starting);
        assert_eq!(status.world_id.as_deref(), Some("x"));
    }

    #[test]
    fn surfaces_rpc_errors_and_rejects_bad_envelopes() {
        let error =
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32010,"message":"world not found"}}"#;
        assert!(matches!(
            parse_world_response(error),
            Err(BridgeError::ControlRpc { code: -32010, .. })
        ));
        let wrong_schema = br#"{"jsonrpc":"2.0","id":1,"result":{"schema_version":2}}"#;
        assert!(parse_world_response(wrong_schema).is_err());
        let wrong_id = br#"{"jsonrpc":"2.0","id":9,"result":{"schema_version":1}}"#;
        assert!(parse_world_response(wrong_id).is_err());
        let missing = require_world(WorldResult {
            schema_version: 1,
            ..WorldResult::default()
        });
        assert!(missing.is_err());
    }
}
