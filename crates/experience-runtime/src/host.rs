//! The host side of the `server` world: generated bindings, per-store state and the imports.

use std::time::Duration;

use anyhow::{Result, bail};
use wasmtime::component::Resource;
use wasmtime::{Engine, Store, StoreLimits, StoreLimitsBuilder};

use crate::limits::{
    EPOCH_PERIOD, MAX_CORE_INSTANCES, MAX_LOG_BYTES, MAX_LOGS, MAX_MEMORIES, MAX_MEMORY_BYTES,
    MAX_TABLE_ELEMENTS,
};

wasmtime::component::bindgen!({
    path: "../experience-sdk/wit",
    world: "server",
    imports: { default: trappable },
    with: { "cinnabar:experience-server/world-access/callback": crate::host::CallbackRes },
});

use cinnabar::experience_server::types::{CallbackInfo, LogLevel, WorldError};
use cinnabar::experience_server::{diagnostics, types, world_access};

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

/// The host value behind a guest's `callback` handle. Handles exist only during a callback;
/// `register` runs outside any, so world access is unavailable there.
pub enum CallbackRes {}

const NO_CALLBACK: &str = "world access is unavailable outside a callback";

/// The data of one store, which serves one `register` or one callback.
pub(crate) struct HostState {
    limits: StoreLimits,
    logs: Logs,
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
        let limits = StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .memories(MAX_MEMORIES)
            .table_elements(MAX_TABLE_ELEMENTS)
            .instances(MAX_CORE_INSTANCES)
            .trap_on_grow_failure(true)
            .build();
        let logs = Logs {
            id: id.to_owned(),
            kept: 0,
            dropped: 0,
            truncated: 0,
        };
        let mut store = Store::new(engine, Self { limits, logs });
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

impl world_access::HostCallback for HostState {
    fn info(&mut self, _: Resource<CallbackRes>) -> Result<CallbackInfo> {
        bail!(NO_CALLBACK)
    }

    fn get_block(
        &mut self,
        _: Resource<CallbackRes>,
        _: BlockPos,
    ) -> Result<Result<String, WorldError>> {
        bail!(NO_CALLBACK)
    }

    fn set_block(
        &mut self,
        _: Resource<CallbackRes>,
        _: BlockPos,
        _: String,
    ) -> Result<Result<(), WorldError>> {
        bail!(NO_CALLBACK)
    }

    fn block_data(
        &mut self,
        _: Resource<CallbackRes>,
        _: BlockPos,
    ) -> Result<Result<Option<Vec<u8>>, WorldError>> {
        bail!(NO_CALLBACK)
    }

    fn set_block_data(
        &mut self,
        _: Resource<CallbackRes>,
        _: BlockPos,
        _: Option<Vec<u8>>,
    ) -> Result<Result<(), WorldError>> {
        bail!(NO_CALLBACK)
    }

    fn tell(
        &mut self,
        _: Resource<CallbackRes>,
        _: String,
        _: String,
    ) -> Result<Result<(), WorldError>> {
        bail!(NO_CALLBACK)
    }

    fn drop(&mut self, _: Resource<CallbackRes>) -> Result<()> {
        bail!(NO_CALLBACK)
    }
}
