//! The host side of the `server` world: generated bindings, per-store state and the imports.

use std::fmt;
use std::time::Duration;

use anyhow::{Result, bail};
use wasmtime::component::{Resource, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder};

use crate::limits::{
    EPOCH_PERIOD, MAX_CORE_INSTANCES, MAX_LOG_BYTES, MAX_LOGS, MAX_MEMORIES, MAX_MEMORY_BYTES,
    MAX_TABLE_ELEMENTS,
};

wasmtime::component::bindgen!({
    path: "../experience-sdk/wit",
    world: "server",
    imports: { default: trappable },
    with: { "cinnabar:experience-server/world-access/callback": crate::callback::CallbackRes },
});

use cinnabar::experience_server::types::{CallbackInfo, LogLevel, WorldError};
use cinnabar::experience_server::{diagnostics, types, world_access};

use crate::callback::CallbackRes;

/// The WIT that `bindgen!` reads; its `package` line is the one source of the world's name and
/// version.
const WIT: &str = include_str!("../../experience-sdk/wit/server.wit");

/// The WIT package, `cinnabar:experience-server@<major.minor.patch>`.
pub(crate) fn wit_package() -> &'static str {
    WIT.lines()
        .find_map(|line| line.strip_prefix("package ")?.strip_suffix(';'))
        .expect("server.wit declares its package")
}

/// The manifest `api` this runtime implements: the WIT package's `major.minor`.
pub(crate) fn api_version() -> &'static str {
    let (_, version) = wit_package()
        .rsplit_once('@')
        .expect("server.wit's package is versioned");
    version.rsplit_once('.').map_or(version, |(api, _)| api)
}

/// A store limit or a per-callback cap was exceeded. A trap with this error fails the callback
/// as `limit`.
#[derive(Debug)]
pub(crate) struct LimitExceeded(pub(crate) String);

impl fmt::Display for LimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for LimitExceeded {}

/// The data of one store, which serves one `register` or one callback.
pub(crate) struct HostState {
    limits: Limiter,
    logs: Logs,
    /// Holds the callback the guest borrows; `register` runs without one.
    pub(crate) table: ResourceTable,
}

impl HostState {
    /// A fresh store for Experience `id` with the store limits, `fuel`, and an epoch deadline
    /// `deadline` from now.
    pub(crate) fn store(
        engine: &Engine,
        id: &str,
        fuel: u64,
        deadline: Duration,
    ) -> Result<Store<Self>> {
        let limits = Limiter(
            StoreLimitsBuilder::new()
                .memory_size(MAX_MEMORY_BYTES)
                .memories(MAX_MEMORIES)
                .table_elements(MAX_TABLE_ELEMENTS)
                .instances(MAX_CORE_INSTANCES)
                .trap_on_grow_failure(true)
                .build(),
        );
        let logs = Logs {
            id: id.to_owned(),
            kept: 0,
            dropped: 0,
            truncated: 0,
        };
        let state = Self {
            limits,
            logs,
            table: ResourceTable::new(),
        };
        let mut store = Store::new(engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(fuel)?;
        store.set_epoch_deadline(epoch_ticks(deadline));
        Ok(store)
    }
}

/// Whole epoch periods in `deadline`, rounded up.
fn epoch_ticks(deadline: Duration) -> u64 {
    let ticks = deadline.as_nanos().div_ceil(EPOCH_PERIOD.as_nanos());
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// The store limits from [`crate::limits`]. Growth they refuse traps with [`LimitExceeded`].
struct Limiter(StoreLimits);

impl ResourceLimiter for Limiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool> {
        self.0
            .memory_growing(current, desired, maximum)
            .map_err(exceeded)
    }

    fn memory_grow_failed(&mut self, error: anyhow::Error) -> Result<()> {
        self.0.memory_grow_failed(error).map_err(exceeded)
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> Result<bool> {
        self.0
            .table_growing(current, desired, maximum)
            .map_err(exceeded)
    }

    fn table_grow_failed(&mut self, error: anyhow::Error) -> Result<()> {
        self.0.table_grow_failed(error).map_err(exceeded)
    }

    fn instances(&self) -> usize {
        self.0.instances()
    }

    fn tables(&self) -> usize {
        self.0.tables()
    }

    fn memories(&self) -> usize {
        self.0.memories()
    }
}

fn exceeded(error: anyhow::Error) -> anyhow::Error {
    LimitExceeded(format!("{error:#}")).into()
}

/// Guest log lines of one store. Logs are not host calls: the first [`MAX_LOGS`] go to stderr,
/// each cut to [`MAX_LOG_BYTES`] at a char boundary, and later ones are dropped. Drops and cuts
/// are counted on stderr when the store ends.
struct Logs {
    id: String,
    kept: usize,
    dropped: usize,
    truncated: usize,
}

impl Logs {
    fn push(&mut self, level: LogLevel, text: &str) {
        if self.kept == MAX_LOGS {
            self.dropped += 1;
            return;
        }
        self.kept += 1;
        let end = text.floor_char_boundary(MAX_LOG_BYTES);
        if end < text.len() {
            self.truncated += 1;
        }
        let level = match level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
        };
        // Debug formatting escapes control characters, so one log stays one stderr line.
        eprintln!("log {} {level} {:?}", self.id, &text[..end]);
    }
}

impl Drop for Logs {
    fn drop(&mut self) {
        if self.dropped > 0 || self.truncated > 0 {
            eprintln!(
                "log {} dropped {} truncated {}",
                self.id, self.dropped, self.truncated
            );
        }
    }
}

impl types::Host for HostState {}

impl diagnostics::Host for HostState {
    fn log(&mut self, level: LogLevel, text: String) -> Result<()> {
        self.logs.push(level, &text);
        Ok(())
    }
}

impl world_access::Host for HostState {}

/// Each method acts on the callback that `ctx` names; see [`CallbackRes`] for the rules.
impl world_access::HostCallback for HostState {
    fn info(&mut self, ctx: Resource<CallbackRes>) -> Result<CallbackInfo> {
        self.table.get_mut(&ctx)?.info()
    }

    fn get_block(
        &mut self,
        ctx: Resource<CallbackRes>,
        pos: BlockPos,
    ) -> Result<Result<String, WorldError>> {
        self.table.get_mut(&ctx)?.get_block(pos.into())
    }

    fn set_block(
        &mut self,
        ctx: Resource<CallbackRes>,
        pos: BlockPos,
        id: String,
    ) -> Result<Result<(), WorldError>> {
        self.table.get_mut(&ctx)?.set_block(pos.into(), id)
    }

    fn block_data(
        &mut self,
        ctx: Resource<CallbackRes>,
        pos: BlockPos,
    ) -> Result<Result<Option<Vec<u8>>, WorldError>> {
        self.table.get_mut(&ctx)?.block_data(pos.into())
    }

    fn set_block_data(
        &mut self,
        ctx: Resource<CallbackRes>,
        pos: BlockPos,
        data: Option<Vec<u8>>,
    ) -> Result<Result<(), WorldError>> {
        self.table.get_mut(&ctx)?.set_block_data(pos.into(), data)
    }

    fn tell(
        &mut self,
        ctx: Resource<CallbackRes>,
        player: String,
        text: String,
    ) -> Result<Result<(), WorldError>> {
        self.table.get_mut(&ctx)?.tell(player, text)
    }

    /// Only reachable for an owned handle, and the guest is only ever lent a callback.
    fn drop(&mut self, _: Resource<CallbackRes>) -> Result<()> {
        bail!("the guest dropped a callback it cannot own")
    }
}
