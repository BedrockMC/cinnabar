//! Host-owned declarative transactions and aggregate reservations.

use crate::{
    manifest::{Permission, Scope, identifier, plain_text},
    policy::*,
    wire::{Channel, Direction, Scalar},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CALLBACK_FUEL: u64 = 100_000;
pub const SESSION_FUEL: u64 = CALLBACK_FUEL * 2;
pub const CALLBACK_INTERVAL_MS: u64 = 34;
pub const MAX_WIDGETS: usize = 128;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub session: String,
    pub bundle: String,
    pub generation: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Widget {
        id: String,
        text: String,
    },
    Screen {
        template: Option<String>,
    },
    Send {
        channel: String,
        schema: u16,
        record: Vec<Scalar>,
    },
    Scene {
        id: u32,
        object: Option<SceneObject>,
    },
    Media {
        id: String,
        operation: MediaOperation,
        position_ms: u64,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaOperation {
    Prepare,
    Play,
    Pause,
    Seek,
    Stop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SceneObject {
    Quad {
        texture: String,
        transform: [f32; 10],
        size: [f32; 2],
    },
    Mesh {
        asset: String,
        transform: [f32; 10],
        triangles: u32,
    },
    Particles {
        effect: String,
        transform: [f32; 10],
        count: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub owner: Principal,
    pub epoch: u64,
    pub commands: Vec<Command>,
}

#[derive(Clone, Debug, Default)]
pub struct Contributions {
    pub widgets: BTreeMap<String, String>,
    pub screen: Option<String>,
    pub scene: BTreeMap<u32, SceneObject>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub scope: Scope,
    pub assets: BTreeSet<String>,
    pub channels: Vec<Channel>,
    pub actions: BTreeSet<String>,
}

impl Capabilities {
    /// Validates a staged operation on both sides of the helper boundary.
    pub fn validate(&self, command: &Command) -> Result<()> {
        let permission = match command {
            Command::Widget { id, text } => {
                ensure!(
                    identifier(id) && (text.is_empty() || plain_text(text, MAX_WIDGET_TEXT_BYTES)),
                    "invalid widget"
                );
                Permission::Ui
            }
            Command::Screen { template } => {
                ensure!(
                    template
                        .as_ref()
                        .is_none_or(|id| self.assets.contains(id) && id.ends_with(".json")),
                    "unknown screen"
                );
                Permission::ModalUi
            }
            Command::Send {
                channel,
                schema,
                record,
            } => {
                let declaration = self
                    .channels
                    .iter()
                    .find(|c| &c.id == channel && c.schema == *schema)
                    .ok_or_else(|| anyhow::anyhow!("undeclared channel"))?;
                declaration.validate(record, Direction::ToServer)?;
                Permission::Messaging
            }
            Command::Scene { object, .. } => {
                if let Some(object) = object {
                    let (asset, transform) = match object {
                        SceneObject::Quad {
                            texture,
                            transform,
                            size,
                        } => {
                            ensure!(
                                size.iter().all(|v| v.is_finite() && *v > 0.0 && *v <= 64.0),
                                "invalid quad size"
                            );
                            (texture, transform)
                        }
                        SceneObject::Mesh {
                            asset,
                            transform,
                            triangles,
                        } => {
                            ensure!(*triangles <= MAX_TRIANGLES, "mesh too large");
                            (asset, transform)
                        }
                        SceneObject::Particles {
                            effect,
                            transform,
                            count,
                        } => {
                            ensure!(*count <= MAX_PARTICLES, "particle budget exceeded");
                            (effect, transform)
                        }
                    };
                    ensure!(self.assets.contains(asset), "unowned scene asset");
                    ensure!(
                        transform.iter().all(|v| v.is_finite()),
                        "nonfinite transform"
                    );
                    ensure!(
                        transform[..3].iter().all(|v| v.abs() <= 30_000_000.0),
                        "translation out of range"
                    );
                    let rotation = transform[3..7].iter().map(|v| v * v).sum::<f32>();
                    ensure!(
                        (rotation - 1.0).abs() <= 0.001,
                        "quaternion must be normalized"
                    );
                    ensure!(
                        transform[7..].iter().all(|v| *v > 0.0 && *v <= 64.0),
                        "invalid scale"
                    );
                }
                Permission::Scene
            }
            Command::Media { id, .. } => {
                ensure!(self.assets.contains(id), "unowned media descriptor");
                Permission::Media
            }
        };
        ensure!(
            self.scope.permissions.contains(&permission),
            "capability denied"
        );
        Ok(())
    }
}

impl Contributions {
    /// Applies all retained changes privately; a rejection preserves the old state.
    pub fn apply(
        &mut self,
        transaction: &Transaction,
        owner: &Principal,
        epoch: u64,
        capabilities: &Capabilities,
    ) -> Result<()> {
        ensure!(
            &transaction.owner == owner && transaction.epoch == epoch,
            "stale or foreign handle"
        );
        ensure!(
            serde_json::to_vec(transaction)?.len() <= MAX_HOST_OUTPUT,
            "transaction too large"
        );
        let mut candidate = self.clone();
        for command in &transaction.commands {
            capabilities.validate(command)?;
            match command {
                Command::Widget { id, text } => {
                    if text.is_empty() {
                        candidate.widgets.remove(id);
                    } else {
                        candidate.widgets.insert(id.clone(), text.clone());
                    }
                }
                Command::Screen { template } => candidate.screen = template.clone(),
                Command::Scene { id, object } => {
                    if let Some(object) = object {
                        candidate.scene.insert(*id, object.clone());
                    } else {
                        candidate.scene.remove(id);
                    }
                }
                Command::Send { .. } | Command::Media { .. } => {}
            }
        }
        ensure!(
            candidate.widgets.len() <= MAX_WIDGETS,
            "widget budget exceeded"
        );
        ensure!(
            candidate.scene.len() <= MAX_DRAWS as usize,
            "draw budget exceeded"
        );
        let mut triangles = 0u64;
        let mut particles = 0u64;
        for object in candidate.scene.values() {
            match object {
                SceneObject::Quad { .. } => triangles += 2,
                SceneObject::Mesh {
                    triangles: count, ..
                } => triangles += u64::from(*count),
                SceneObject::Particles { count, .. } => particles += u64::from(*count),
            }
        }
        ensure!(
            triangles <= u64::from(MAX_TRIANGLES) && particles <= u64::from(MAX_PARTICLES),
            "scene budget exceeded"
        );
        *self = candidate;
        Ok(())
    }
}

#[derive(Default)]
pub struct Budget {
    reservations: BTreeMap<Principal, (u64, u64)>,
    quarantined: BTreeSet<Principal>,
    fuel: u64,
}

impl Budget {
    /// Reserves worst-case guest and GPU memory before launching a helper.
    pub fn reserve(&mut self, owner: Principal, memory: u64, gpu: u64) -> Result<()> {
        ensure!(
            !self.quarantined.contains(&owner) && !self.reservations.contains_key(&owner),
            "instance unavailable"
        );
        ensure!(
            self.reservations.len() < MAX_BUNDLES && memory <= MAX_GUEST_MEMORY,
            "bundle limit exceeded"
        );
        let memory_total: u64 = self.reservations.values().map(|v| v.0).sum();
        let gpu_total: u64 = self.reservations.values().map(|v| v.1).sum();
        ensure!(
            memory <= MAX_SESSION_MEMORY - memory_total && gpu <= MAX_GPU_BYTES - gpu_total,
            "aggregate memory budget exceeded"
        );
        self.reservations.insert(owner, (memory, gpu));
        Ok(())
    }

    /// Starts one aggregate scheduling slice, independent of bundle count.
    pub fn begin_slice(&mut self) {
        self.fuel = SESSION_FUEL;
    }

    /// Reports whether a reserved guest can run without exceeding this slice.
    pub fn can_dispatch(&self, owner: &Principal) -> bool {
        self.reservations.contains_key(owner) && self.fuel >= CALLBACK_FUEL
    }

    /// Charges the full callback allowance before scheduling guest work.
    pub fn dispatch(&mut self, owner: &Principal) -> Result<u64> {
        ensure!(self.can_dispatch(owner), "callback deferred");
        self.fuel -= CALLBACK_FUEL;
        Ok(CALLBACK_FUEL)
    }

    /// A crashed bundle cannot be restarted until the whole session is replaced.
    pub fn quarantine(&mut self, owner: &Principal) {
        self.reservations.remove(owner);
        self.quarantined.insert(owner.clone());
    }
}
