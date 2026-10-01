//! The adapter protocol: message types, framing and the golden fixtures the Go adapter checks.
//!
//! A frame is a 4-byte little-endian length followed by that many bytes of JSON. Bytes inside
//! messages are lowercase hex (see [`crate::hex`]).

use std::io::{self, ErrorKind, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::hex;
use crate::limits::MAX_FRAME_BYTES;

pub const PROTOCOL_VERSION: u32 = 1;

const _: () = assert!(MAX_FRAME_BYTES <= u32::MAX as usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Face {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Info {
    pub world_id: String,
    pub dimension_id: String,
    pub tick: u64,
    pub event_sequence: u64,
}

/// One snapshot cell. `id` is empty when the cell is not loaded; `data` is lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cell {
    pub pos: BlockPos,
    pub loaded: bool,
    pub id: String,
    pub owned: bool,
    pub data: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cause {
    Player,
    Guest,
    Environment,
}

/// A committed block change; `previous_data` is lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub pos: BlockPos,
    pub actor: Option<String>,
    pub cause: Cause,
    pub before_id: String,
    pub after_id: String,
    pub previous_data: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Call {
    Place {
        change: Change,
    },
    Break {
        change: Change,
    },
    Interact {
        player: String,
        pos: BlockPos,
        face: Face,
    },
    Neighbor {
        pos: BlockPos,
        neighbor: BlockPos,
    },
}

/// Adapter → runtime. One request is decoded per frame and never stored in bulk, so the large
/// `Callback` variant stays inline.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Load {
        dir: String,
    },
    Callback {
        seq: u64,
        info: Info,
        actor: Option<String>,
        world_min_y: i32,
        world_max_y: i32,
        data_budget: u64,
        snapshot: Vec<Cell>,
        call: Call,
    },
    Shutdown,
}

/// A texture binding; `path` is absolute and validated by the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Texture {
    pub slot: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mining {
    Unbreakable,
    Breakable { hardness: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDef {
    pub id: String,
    pub display_name: String,
    pub textures: Vec<Texture>,
    pub mining: Mining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailKind {
    Trap,
    Fuel,
    Deadline,
    Limit,
}

/// A staged world operation; `data` is lowercase hex, `None` clears it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    SetBlock { pos: BlockPos, id: String },
    SetBlockData { pos: BlockPos, data: Option<String> },
    Tell { player: String, text: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    Committed {
        ops: Vec<Op>,
    },
    /// A guest error; not a strike.
    Rejected {
        reason: String,
    },
    /// A strike against the Experience.
    Failed {
        kind: FailKind,
        reason: String,
    },
}

/// Runtime → adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    Loaded {
        protocol: u32,
        id: String,
        version: String,
        blocks: Vec<BlockDef>,
    },
    LoadFailed {
        reason: String,
    },
    Result {
        seq: u64,
        outcome: Outcome,
    },
}

/// Writes one frame and flushes. A message whose JSON exceeds [`MAX_FRAME_BYTES`] is rejected
/// with [`ErrorKind::InvalidInput`] before anything is written.
pub fn write_frame(w: &mut impl Write, msg: &impl Serialize) -> io::Result<()> {
    let mut frame = vec![0; 4];
    serde_json::to_writer(&mut frame, msg).map_err(io::Error::other)?;
    let length = frame.len() - 4;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("frame of {length} bytes exceeds {MAX_FRAME_BYTES}"),
        ));
    }
    frame[..4].copy_from_slice(&(length as u32).to_le_bytes());
    w.write_all(&frame)?;
    w.flush()
}

/// Reads one frame. Returns `Ok(None)` on a clean EOF before the length; a truncated frame is
/// [`ErrorKind::UnexpectedEof`], and an oversized or undecodable one is [`ErrorKind::InvalidData`].
pub fn read_frame<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<Option<T>> {
    let mut prefix = [0; 4];
    let mut filled = 0;
    while filled < prefix.len() {
        match r.read(&mut prefix[filled..]) {
            Ok(0) if filled == 0 => return Ok(None),
            Ok(0) => {
                return Err(io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "frame length is truncated",
                ));
            }
            Ok(read) => filled += read,
            Err(err) if err.kind() == ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    let length = u32::from_le_bytes(prefix) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!("frame of {length} bytes exceeds {MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0; length];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|err| io::Error::new(ErrorKind::InvalidData, err))
}

/// Constants the Go adapter must agree with.
#[derive(Serialize)]
struct Limits {
    max_frame_bytes: usize,
    protocol: u32,
}

/// The golden fixtures, as `(file stem, pretty JSON with a trailing newline)`: one per
/// request, response, outcome, op and call variant, plus `limits`.
pub fn fixtures() -> Vec<(&'static str, String)> {
    fn pretty(msg: &impl Serialize) -> String {
        serde_json::to_string_pretty(msg).expect("fixtures serialize") + "\n"
    }
    let player = "6f1c3c2e-5b7a-4d3e-9a51-0c8e2f4b7d10";
    let controller = BlockPos {
        x: 12,
        y: 64,
        z: -7,
    };
    let neighbor = BlockPos {
        x: 13,
        y: 64,
        z: -7,
    };
    let callback = |seq: u64, actor: Option<&str>, call: Call| Request::Callback {
        seq,
        info: Info {
            world_id: "world".to_owned(),
            dimension_id: "overworld".to_owned(),
            tick: 48_213,
            event_sequence: seq + 900,
        },
        actor: actor.map(str::to_owned),
        world_min_y: -64,
        world_max_y: 319,
        data_budget: 4096,
        snapshot: vec![
            Cell {
                pos: controller,
                loaded: true,
                id: "benergistics:controller".to_owned(),
                owned: true,
                data: Some(hex::encode(&[0x01, 0x00, 0xff])),
            },
            Cell {
                pos: neighbor,
                loaded: true,
                id: "minecraft:stone".to_owned(),
                owned: false,
                data: None,
            },
            Cell {
                pos: BlockPos {
                    x: 12,
                    y: 64,
                    z: 400,
                },
                loaded: false,
                id: String::new(),
                owned: false,
                data: None,
            },
        ],
        call,
    };
    let result = |seq: u64, outcome: Outcome| Response::Result { seq, outcome };
    let texture = |slot: &str, file: &str| Texture {
        slot: slot.to_owned(),
        path: format!("/srv/experiences/benergistics/assets/{file}"),
    };

    vec![
        (
            "request_load",
            pretty(&Request::Load {
                dir: "/srv/experiences/benergistics".to_owned(),
            }),
        ),
        (
            "request_callback_place",
            pretty(&callback(
                1,
                Some(player),
                Call::Place {
                    change: Change {
                        pos: controller,
                        actor: Some(player.to_owned()),
                        cause: Cause::Player,
                        before_id: "minecraft:air".to_owned(),
                        after_id: "benergistics:controller".to_owned(),
                        previous_data: None,
                    },
                },
            )),
        ),
        (
            "request_callback_break",
            pretty(&callback(
                2,
                None,
                Call::Break {
                    change: Change {
                        pos: controller,
                        actor: None,
                        cause: Cause::Environment,
                        before_id: "benergistics:controller".to_owned(),
                        after_id: "minecraft:air".to_owned(),
                        previous_data: Some(hex::encode(&[0x01, 0x00, 0xff])),
                    },
                },
            )),
        ),
        (
            "request_callback_interact",
            pretty(&callback(
                3,
                Some(player),
                Call::Interact {
                    player: player.to_owned(),
                    pos: controller,
                    face: Face::North,
                },
            )),
        ),
        (
            "request_callback_neighbor",
            pretty(&callback(
                4,
                None,
                Call::Neighbor {
                    pos: controller,
                    neighbor,
                },
            )),
        ),
        ("request_shutdown", pretty(&Request::Shutdown)),
        (
            "response_loaded",
            pretty(&Response::Loaded {
                protocol: PROTOCOL_VERSION,
                id: "benergistics".to_owned(),
                version: "0.1.0".to_owned(),
                blocks: vec![
                    BlockDef {
                        id: "benergistics:controller".to_owned(),
                        display_name: "ME Controller".to_owned(),
                        textures: vec![
                            texture("*", "controller.png"),
                            texture("up", "controller_powered.png"),
                        ],
                        mining: Mining::Breakable { hardness: 1.5 },
                    },
                    BlockDef {
                        id: "benergistics:creative_energy_cell".to_owned(),
                        display_name: "Creative Energy Cell".to_owned(),
                        textures: vec![texture("*", "creative_energy_cell.png")],
                        mining: Mining::Unbreakable,
                    },
                ],
            }),
        ),
        (
            "response_load_failed",
            pretty(&Response::LoadFailed {
                reason: "server.wasm: SHA-256 does not match experience.toml".to_owned(),
            }),
        ),
        (
            "response_result_committed",
            pretty(&result(
                1,
                Outcome::Committed {
                    ops: vec![
                        Op::SetBlock {
                            pos: neighbor,
                            id: "benergistics:creative_energy_cell".to_owned(),
                        },
                        Op::SetBlockData {
                            pos: controller,
                            data: Some(hex::encode(&[0x02, 0x10, 0xab])),
                        },
                        Op::SetBlockData {
                            pos: neighbor,
                            data: None,
                        },
                        Op::Tell {
                            player: player.to_owned(),
                            text: "Network online".to_owned(),
                        },
                    ],
                },
            )),
        ),
        (
            "response_result_rejected",
            pretty(&result(
                2,
                Outcome::Rejected {
                    reason: "a controller already powers this network".to_owned(),
                },
            )),
        ),
        (
            "response_result_failed",
            pretty(&result(
                3,
                Outcome::Failed {
                    kind: FailKind::Fuel,
                    reason: "callback exhausted its fuel".to_owned(),
                },
            )),
        ),
        (
            "limits",
            pretty(&Limits {
                max_frame_bytes: MAX_FRAME_BYTES,
                protocol: PROTOCOL_VERSION,
            }),
        ),
    ]
}
