//! Transactional server-bundle host. Production callers must use a restricted helper.

use anyhow::{Result, ensure};
use server_experience::{policy::*, runtime::{Capabilities, Command, MediaOperation, Principal, Transaction, CALLBACK_FUEL}};
use std::collections::BTreeSet;
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder, component::{Component, HasSelf, Linker}};

wasmtime::component::bindgen!({
    path: "../mod-api/wit", world: "server-bundle", imports: { default: trappable },
});

struct State {
    limits: StoreLimits,
    owner: Principal,
    epoch: u64,
    capabilities: Capabilities,
    actions: BTreeSet<String>,
    commands: Vec<Command>,
    bytes: usize,
    calls: usize,
}

impl State {
    /// Stops host-call floods even when the guest repeatedly ignores denied results.
    fn charge(&mut self) -> Result<()> {
        self.calls += 1;
        ensure!(self.calls <= 256, "host-call limit exceeded");
        Ok(())
    }

    /// Stages output privately; denied operations never reach the engine.
    fn stage(&mut self, command: Command) -> Result<Result<(), String>> {
        self.charge()?;
        if let Err(error) = self.capabilities.validate(&command) { return Ok(Err(error.to_string())); }
        let size = serde_json::to_vec(&command)?.len();
        ensure!(size <= MAX_HOST_OUTPUT - self.bytes, "host output budget exceeded");
        self.bytes += size;
        self.commands.push(command);
        Ok(Ok(()))
    }
}

impl cinnabar::server_experience::ui::Host for State {
    /// Stages bounded label text for an owned widget.
    fn set_widget(&mut self, id: String, text: String) -> Result<Result<(), String>> {
        self.stage(Command::Widget { id, text })
    }

    /// Requests only a signed template from this bundle's asset set.
    fn open_screen(&mut self, template: Option<String>) -> Result<Result<(), String>> {
        self.stage(Command::Screen { template })
    }
}

impl cinnabar::server_experience::input::Host for State {
    /// Reads a declared action edge; the caller supplies no keyboard state.
    fn pressed(&mut self, action: String) -> Result<bool> {
        self.charge()?;
        Ok(self.capabilities.scope.permissions.contains(&server_experience::manifest::Permission::Input)
            && self.capabilities.actions.contains(&action) && self.actions.contains(&action))
    }
}

impl cinnabar::server_experience::messaging::Host for State {
    /// Parses and validates the signed channel record before queuing it.
    fn send(&mut self, channel: String, schema: u16, record_json: Vec<u8>) -> Result<Result<(), String>> {
        ensure!(record_json.len() <= MAX_PAYLOAD_BYTES, "message too large");
        let record = serde_json::from_slice(&record_json)?;
        self.stage(Command::Send { channel, schema, record })
    }
}

impl cinnabar::server_experience::scene::Host for State {
    /// Accepts a bounded declarative object, never a native renderer handle.
    fn put(&mut self, id: u32, object_json: Option<Vec<u8>>) -> Result<Result<(), String>> {
        let object = object_json.map(|bytes| {
            ensure!(bytes.len() <= MAX_PAYLOAD_BYTES, "scene object too large");
            Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
        }).transpose()?;
        self.stage(Command::Scene { id, object })
    }
}

impl cinnabar::server_experience::media::Host for State {
    /// Refers to an approved media descriptor, without exposing fetch APIs.
    fn control(&mut self, id: String, op: cinnabar::server_experience::media::Operation, position_ms: u64) -> Result<Result<(), String>> {
        use cinnabar::server_experience::media::Operation;
        let operation = match op {
            Operation::Prepare => MediaOperation::Prepare,
            Operation::Play => MediaOperation::Play,
            Operation::Pause => MediaOperation::Pause,
            Operation::Seek => MediaOperation::Seek,
            Operation::Stop => MediaOperation::Stop,
        };
        self.stage(Command::Media { id, operation, position_ms })
    }
}

pub struct BundleHost {
    store: Store<State>,
    guest: ServerBundle,
    active: bool,
}

impl BundleHost {
    /// Unsafe for production remote code: this explicit developer path has no OS sandbox.
    pub fn developer_in_process(bytes: &[u8], owner: Principal, capabilities: Capabilities, epoch: u64) -> Result<Self> {
        ensure!(std::env::var(DEVELOPER_ENV).as_deref() == Ok("1"), "in-process server code is developer-only");
        Self::instantiate(bytes, owner, capabilities, epoch)
    }

    /// Compiles inside the helper, with no WASI or precompiled native cache input.
    pub(crate) fn instantiate(bytes: &[u8], owner: Principal, capabilities: Capabilities, epoch: u64) -> Result<Self> {
        ensure!(bytes.len() <= MAX_COMPONENT_BYTES && bytes.starts_with(b"\0asm"), "invalid component bytes");
        capabilities.scope.validate()?;
        ensure!(capabilities.scope.memory_bytes > 0 && capabilities.scope.memory_bytes <= MAX_GUEST_MEMORY, "invalid guest memory limit");
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true).max_wasm_stack(256 * 1024);
        let engine = Engine::new(&config)?;
        let component = Component::new(&engine, bytes)?;
        let mut linker = Linker::new(&engine);
        ServerBundle::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut State| state)?;
        let state = State {
            limits: StoreLimitsBuilder::new()
                .memory_size(capabilities.scope.memory_bytes as usize)
                .table_elements(4096).instances(16).memories(1).tables(2)
                .trap_on_grow_failure(true).build(),
            owner, epoch, capabilities, actions: BTreeSet::new(), commands: Vec::new(), bytes: 0, calls: 0,
        };
        let mut store = Store::new(&engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(CALLBACK_FUEL)?;
        let guest = ServerBundle::instantiate(&mut store, &component, &linker)?;
        guest.call_init(&mut store)?;
        Ok(Self { store, guest, active: true })
    }

    /// Runs one fuel-bounded callback; a trap discards all output and quarantines this instance.
    pub fn dispatch(&mut self, channel: &str, record: &[u8], actions: BTreeSet<String>, epoch: u64) -> Result<Transaction> {
        ensure!(self.active, "bundle quarantined");
        ensure!(record.len() <= MAX_PAYLOAD_BYTES && channel.len() <= 96, "event too large");
        let state = self.store.data_mut();
        state.commands.clear();
        state.bytes = 0;
        state.calls = 0;
        state.actions = actions;
        state.epoch = epoch;
        self.store.set_fuel(CALLBACK_FUEL)?;
        if let Err(error) = self.guest.call_dispatch(&mut self.store, channel, record) {
            self.active = false;
            self.store.data_mut().commands.clear();
            return Err(error);
        }
        Ok(self.take_transaction())
    }

    /// Takes only successfully returned initialization or callback output.
    pub fn take_transaction(&mut self) -> Transaction {
        let state = self.store.data_mut();
        Transaction { owner: state.owner.clone(), epoch: state.epoch, commands: std::mem::take(&mut state.commands) }
    }
}
