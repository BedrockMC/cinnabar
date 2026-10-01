//! Developer helper supervision; only implemented adapters receive grants.

use anyhow::{Result, ensure};
use mod_host::helper::{Dispatch, Helper};
use server_experience::{
    bundle::VerifiedBundle,
    manifest::implemented_permissions,
    negotiation::Grant,
    policy::*,
    runtime::{Budget, CALLBACK_INTERVAL_MS, Capabilities, Command, Contributions, Principal},
    session::Control,
    wire::{Envelope, Ingress, RateLimit},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

struct Instance {
    helper: Option<Helper>,
    capabilities: Capabilities,
    owner: Principal,
    contributions: Contributions,
    busy: bool,
}

pub(super) struct Live {
    grant: Grant,
    instances: BTreeMap<String, Instance>,
    budget: Budget,
    ingress: Ingress,
    egress: RateLimit,
    sequence: u64,
    slice_ms: u64,
    ready: bool,
    epoch: u64,
}

impl Live {
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
                generation: 1,
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
                channels: bundle.manifest.channels.clone(),
                actions: bundle.manifest.actions.clone(),
            };
            budget.reserve(
                owner.clone(),
                capabilities.scope.memory_bytes,
                capabilities.scope.gpu_bytes,
            )?;
            let helper = if let Some(bytes) = bundle.component() {
                budget.dispatch(&owner)?;
                Some(Helper::spawn_developer(
                    executable,
                    bytes,
                    owner.clone(),
                    capabilities.clone(),
                    epoch,
                )?)
            } else {
                None
            };
            let busy = helper.is_some();
            instances.insert(
                owner.bundle.clone(),
                Instance {
                    helper,
                    capabilities,
                    owner,
                    contributions: Contributions::default(),
                    busy,
                },
            );
        }
        Ok(Self {
            grant,
            instances,
            budget,
            ingress: Ingress::new(now_ms),
            egress: RateLimit::new(now_ms),
            sequence: 1,
            slice_ms: now_ms,
            ready: false,
            epoch,
        })
    }

    /// Publishes complete transactions only; failure revokes every contribution in this preview.
    pub(super) fn poll(&mut self, epoch: u64, now_ms: u64) -> Result<Vec<Vec<u8>>> {
        ensure!(
            epoch == self.epoch,
            "world epoch changed; extension snapshot required"
        );
        let mut sends = Vec::new();
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
            instance.contributions.apply(
                &transaction,
                &instance.owner,
                epoch,
                &instance.capabilities,
            )?;
            for command in transaction.commands {
                if let Command::Send {
                    channel,
                    schema,
                    record,
                } = command
                {
                    sends.push(Envelope {
                        version: WIRE_VERSION,
                        session: self.grant.session.clone(),
                        connection: self.grant.connection.clone(),
                        subclient: self.grant.subclient,
                        bundle: instance.owner.bundle.clone(),
                        generation: instance.owner.generation,
                        channel,
                        schema,
                        sequence: self.sequence,
                        world_epoch: epoch,
                        payload: record,
                    });
                    self.sequence = self
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("outbound sequence exhausted"))?;
                }
            }
        }
        let mut packets = Vec::new();
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
                generation: 1,
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
        ensure!(
            self.ready || sends.is_empty(),
            "guest sent before all bundles were ready"
        );
        for send in sends {
            let bytes = serde_json::to_vec(&send)?;
            self.egress.charge(bytes.len(), now_ms)?;
            packets.push(bytes);
        }
        if now_ms.saturating_sub(self.slice_ms) >= CALLBACK_INTERVAL_MS {
            self.slice_ms = now_ms;
            self.budget.begin_slice();
            while let Some(message) = self.ingress.pop(u64::MAX, epoch) {
                let instance = self
                    .instances
                    .get_mut(&message.bundle)
                    .ok_or_else(|| anyhow::anyhow!("unknown bundle"))?;
                ensure!(
                    !instance.busy,
                    "bundle event queue requires resynchronization"
                );
                self.budget.dispatch(&instance.owner)?;
                if let Some(helper) = &mut instance.helper {
                    helper.dispatch(Dispatch {
                        channel: message.channel,
                        record: serde_json::to_vec(&message.payload)?,
                        actions: BTreeSet::new(),
                        epoch,
                    })?;
                    instance.busy = true;
                }
            }
        }
        Ok(packets)
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
