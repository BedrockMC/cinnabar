//! Developer helper supervision; only implemented adapters receive grants.

use super::worker::Worker;
use anyhow::{Result, ensure};
use mod_host::helper::{Dispatch, Event, Helper};
use server_experience::{
    bundle::VerifiedBundle,
    manifest::implemented_permissions,
    negotiation::Grant,
    policy::*,
    runtime::{Budget, CALLBACK_INTERVAL_MS, Capabilities, Command, Contributions, Principal},
    screen,
    session::Control,
    wire::{self, Envelope, Ingress, RateLimit},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    sync::Arc,
};

mod modal;

struct Instance<H> {
    helper: Option<H>,
    component: Option<Vec<u8>>,
    capabilities: Capabilities,
    owner: Principal,
    contributions: Contributions,
    busy: bool,
    /// The bundle's verified templates and textures, shared with the modal presenter.
    files: Arc<screen::Files>,
    /// When its modal last opened, so the most recent one draws on top.
    opened: u64,
    /// Host callbacks (modal actions) waiting for the helper, oldest first.
    events: VecDeque<Event>,
    /// The world epoch of its pending callback, which that callback's transaction carries.
    epoch: u64,
}

pub(super) struct Live<H = Helper> {
    grant: Grant,
    instances: BTreeMap<String, Instance<H>>,
    executable: PathBuf,
    pending_sends: VecDeque<Vec<u8>>,
    pending_send_bytes: usize,
    budget: Budget,
    ingress: Ingress,
    egress: RateLimit,
    sequence: u64,
    slice_ms: u64,
    ready: bool,
    epoch: u64,
    /// Counts modal openings across bundles.
    modal_order: u64,
}

impl<H: Worker> Live<H> {
    /// Launches only developer helpers; unsupported required presentation remains denied.
    pub(super) fn start(
        grant: Grant,
        bundles: Vec<VerifiedBundle>,
        epoch: u64,
        now_ms: u64,
        executable: &Path,
    ) -> Result<Self> {
        ensure!(
            bundles
                .iter()
                .map(|bundle| bundle.manifest.channels.len())
                .sum::<usize>()
                <= MAX_CHANNELS,
            "aggregate channel limit exceeded"
        );
        let mut budget = Budget::default();
        budget.begin_slice();
        let mut instances = BTreeMap::new();
        for bundle in bundles {
            let owner = Principal {
                session: grant.session.clone(),
                bundle: bundle.manifest.id.clone(),
                generation: INITIAL_BUNDLE_GENERATION,
            };
            let mut scope = grant.offer.offer.scope.clone();
            scope.permissions = bundle.manifest.permissions.clone();
            scope
                .permissions
                .retain(|permission| implemented_permissions().contains(permission));
            let count = grant.offer.offer.packages.len() as u64;
            scope.memory_bytes = (scope.memory_bytes / count).min(MAX_GUEST_MEMORY);
            scope.gpu_bytes /= count;
            let capabilities = Capabilities {
                scope,
                assets: bundle.paths().map(str::to_owned).collect(),
                templates: bundle.manifest.templates.clone(),
                channels: bundle.manifest.channels.clone(),
                actions: bundle.manifest.actions.clone(),
            };
            budget.reserve(
                owner.clone(),
                capabilities.scope.memory_bytes,
                capabilities.scope.gpu_bytes,
            )?;
            let (component, files) = bundle.into_runtime();
            let busy = component.is_some();
            instances.insert(
                owner.bundle.clone(),
                Instance {
                    helper: None,
                    component,
                    capabilities,
                    owner,
                    contributions: Contributions::default(),
                    busy,
                    files: Arc::new(files),
                    opened: 0,
                    events: VecDeque::new(),
                    epoch,
                },
            );
        }
        let mut live = Self {
            grant,
            instances,
            executable: executable.to_owned(),
            pending_sends: VecDeque::new(),
            pending_send_bytes: 0,
            budget,
            ingress: Ingress::new(now_ms),
            egress: RateLimit::new(now_ms),
            sequence: 1,
            slice_ms: now_ms,
            ready: false,
            epoch,
            modal_order: 0,
        };
        live.initialize()?;
        Ok(live)
    }

    /// Publishes complete transactions only; failure revokes every contribution in this preview.
    pub(super) fn poll(&mut self, epoch: u64, now_ms: u64) -> Result<Vec<Vec<u8>>> {
        let mut packets = Vec::new();
        if epoch != self.epoch {
            self.change_epoch(epoch, now_ms, &mut packets)?;
        }
        if now_ms.saturating_sub(self.slice_ms) >= CALLBACK_INTERVAL_MS {
            self.slice_ms = now_ms;
            self.budget.begin_slice();
        }
        self.initialize()?;

        for instance in self.instances.values_mut() {
            let Some(helper) = &mut instance.helper else {
                continue;
            };
            let Some(result) = helper.poll() else {
                continue;
            };
            instance.busy = false;
            let transaction = match result {
                Ok(transaction) => transaction,
                Err(error) => {
                    self.budget.quarantine(&instance.owner);
                    instance.contributions = Contributions::default();
                    return Err(error);
                }
            };
            // A callback that began before an epoch change still publishes; its sends carry the
            // epoch it began in, which the server drops and counts.
            instance.contributions.apply(
                &transaction,
                &instance.owner,
                instance.epoch,
                &instance.capabilities,
            )?;
            modal::note_opened(instance, &transaction, &mut self.modal_order);
            for command in transaction.commands {
                if let Command::Send {
                    channel,
                    schema,
                    record,
                } = command
                {
                    let send = Envelope {
                        version: self.grant.wire.version,
                        session: self.grant.session.clone(),
                        connection: self.grant.connection.clone(),
                        subclient: self.grant.subclient,
                        bundle: instance.owner.bundle.clone(),
                        generation: instance.owner.generation,
                        channel,
                        schema,
                        sequence: self.sequence,
                        world_epoch: instance.epoch,
                        payload: record,
                    };
                    for bytes in wire::encode(&send, &self.grant.wire)? {
                        ensure!(
                            self.pending_sends.len() < MAX_QUEUE_MESSAGES
                                && bytes.len() <= MAX_QUEUE_BYTES - self.pending_send_bytes,
                            "outbound initialization queue overflow"
                        );
                        self.pending_send_bytes += bytes.len();
                        self.pending_sends.push_back(bytes);
                    }
                    self.sequence = self
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("outbound sequence exhausted"))?;
                }
            }
        }
        if !self.ready && self.instances.values().all(|instance| !instance.busy) {
            self.ready = true;
            packets.push(serde_json::to_vec(&Control::Ready {
                session: self.grant.session.clone(),
                packages: self
                    .grant
                    .offer
                    .offer
                    .packages
                    .iter()
                    .map(|p| p.digest.clone())
                    .collect(),
                generation: INITIAL_BUNDLE_GENERATION,
                permissions: self
                    .instances
                    .iter()
                    .map(|(id, instance)| {
                        (id.clone(), instance.capabilities.scope.permissions.clone())
                    })
                    .collect(),
                world_epoch: self.epoch,
            })?);
        }
        if self.ready {
            while let Some(bytes) = self.pending_sends.pop_front() {
                self.pending_send_bytes -= bytes.len();
                self.egress.charge(bytes.len(), now_ms)?;
                packets.push(bytes);
            }
            self.deliver_events(epoch)?;
            while let Some(message) = self.ingress.peek(u64::MAX, epoch) {
                let instance = self
                    .instances
                    .get_mut(&message.bundle)
                    .ok_or_else(|| anyhow::anyhow!("unknown bundle"))?;
                if instance.busy || !self.budget.can_dispatch(&instance.owner) {
                    break;
                }
                let message = self.ingress.pop(u64::MAX, epoch).expect("front checked");
                self.budget.dispatch(&instance.owner)?;
                if let Some(helper) = &mut instance.helper {
                    helper.dispatch(Dispatch {
                        event: Event::Message {
                            channel: message.channel,
                            record: serde_json::to_vec(&message.payload)?,
                        },
                        epoch,
                    })?;
                    instance.busy = true;
                    instance.epoch = epoch;
                }
            }
        }
        Ok(packets)
    }

    /// Keeps a wire v2 runtime through a world epoch change: once Ready has named the old epoch,
    /// an `epoch` control names the new one ahead of every later send, and each guest is called
    /// back to resend its state. A v1 session cannot continue.
    fn change_epoch(&mut self, epoch: u64, now_ms: u64, packets: &mut Vec<Vec<u8>>) -> Result<()> {
        ensure!(
            self.grant.wire.version != WIRE_VERSION,
            "world epoch changed; extension snapshot required"
        );
        self.epoch = epoch;
        if self.ready {
            let control = serde_json::to_vec(&Control::Epoch {
                session: self.grant.session.clone(),
                world_epoch: epoch,
            })?;
            self.egress.charge(control.len(), now_ms)?;
            packets.push(control);
        }
        for instance in self.instances.values_mut() {
            let guest = instance.helper.is_some() || instance.component.is_some();
            if guest
                && !instance
                    .events
                    .iter()
                    .any(|event| matches!(event, Event::Epoch))
            {
                instance.events.push_back(Event::Epoch);
            }
        }
        Ok(())
    }

    /// Starts pending initializers only when the aggregate callback slice has room.
    fn initialize(&mut self) -> Result<()> {
        for instance in self.instances.values_mut() {
            if instance.component.is_none() || !self.budget.can_dispatch(&instance.owner) {
                continue;
            }
            self.budget.dispatch(&instance.owner)?;
            let component = instance.component.take().expect("component checked");
            instance.helper = Some(H::spawn(
                &self.executable,
                component,
                instance.owner.clone(),
                instance.capabilities.clone(),
                self.epoch,
            )?);
            instance.epoch = self.epoch;
        }
        Ok(())
    }

    /// Applies aggregate limits and signed schemas before guest dispatch.
    pub(super) fn receive(&mut self, bytes: &[u8], now_ms: u64) -> Result<()> {
        ensure!(self.ready, "runtime message before readiness");
        let instances = &self.instances;
        self.ingress.receive(bytes, now_ms, 0, &self.grant, |id| {
            instances.get(id).map(|instance| &instance.capabilities)
        })
    }

    /// Uses only host-owned status text in the persistent execution indicator.
    pub(super) fn text(&self) -> String {
        "Cinnabar: server code running (developer helper). F9: disable".into()
    }

    /// Limits remote text separately from the trusted execution indicator.
    pub(super) fn labels(&self) -> String {
        let labels = self
            .instances
            .values()
            .flat_map(|instance| instance.contributions.widgets.values())
            .take(8)
            .cloned()
            .collect::<Vec<_>>()
            .join(" | ");
        if labels.is_empty() {
            return String::new();
        }
        format!(
            "Server widgets: {}",
            labels.chars().take(256).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests;
