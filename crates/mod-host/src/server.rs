//! Transactional server-bundle host. Production callers must use a restricted helper.

use crate::helper::Event;
use anyhow::{Result, ensure};
use server_experience::{
    policy::*,
    runtime::{CALLBACK_FUEL, Capabilities, Command, MediaOperation, Principal, Transaction},
    screen,
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Func, HasSelf, Instance, Linker},
};

wasmtime::component::bindgen!({
    path: "../experience-sdk/wit/client",
    world: "server-bundle",
    imports: { default: trappable },
});

struct State {
    limits: StoreLimits,
    owner: Principal,
    epoch: u64,
    capabilities: Capabilities,
    /// The declared action this callback delivers, the only one `input.pressed` reports.
    action: Option<String>,
    commands: Vec<Command>,
    bytes: usize,
    calls: usize,
}

impl State {
    /// Reserves the serialized owner, epoch and empty command array before guest output.
    fn begin_output(&mut self) -> Result<()> {
        self.commands.clear();
        self.calls = 0;
        self.bytes = serde_json::to_vec(&Transaction {
            owner: self.owner.clone(),
            epoch: self.epoch,
            commands: Vec::new(),
        })?
        .len();
        ensure!(
            self.bytes <= MAX_HOST_OUTPUT,
            "host output envelope too large"
        );
        Ok(())
    }

    /// Stops host-call floods even when the guest repeatedly ignores denied results.
    fn charge(&mut self) -> Result<()> {
        self.calls += 1;
        ensure!(self.calls <= 256, "host-call limit exceeded");
        Ok(())
    }

    /// Stages output privately; denied operations never reach the engine.
    fn stage(&mut self, command: Command) -> Result<Result<(), String>> {
        self.charge()?;
        if let Err(error) = self.capabilities.validate(&command) {
            return Ok(Err(error.to_string()));
        }
        let size = serde_json::to_vec(&command)?.len() + usize::from(!self.commands.is_empty());
        if size > MAX_HOST_OUTPUT - self.bytes {
            return Ok(Err("host output budget exceeded".into()));
        }
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

    /// Opens only a template the signed manifest indexes; `None` closes the modal.
    fn open_screen(&mut self, template: Option<String>) -> Result<Result<(), String>> {
        self.stage(Command::Screen { template })
    }

    fn close_screen(&mut self) -> Result<Result<(), String>> {
        self.stage(Command::Screen { template: None })
    }

    /// Parses rows within the output budget; malformed rows are denied, not trapped.
    fn set_collection(&mut self, name: String, rows_json: Vec<u8>) -> Result<Result<(), String>> {
        if rows_json.len() > MAX_HOST_OUTPUT {
            self.charge()?;
            return Ok(Err("collection too large".into()));
        }
        match serde_json::from_slice(&rows_json) {
            Ok(rows) => self.stage(Command::Collection { name, rows }),
            Err(error) => {
                self.charge()?;
                Ok(Err(format!("invalid rows: {error}")))
            }
        }
    }

    fn set_value(
        &mut self,
        name: String,
        value: cinnabar::server_experience::ui::Value,
    ) -> Result<Result<(), String>> {
        use cinnabar::server_experience::ui::Value;
        let value = match value {
            Value::Boolean(value) => screen::Value::Bool(value),
            Value::Integer(value) => screen::Value::Integer(value),
            Value::Number(value) => screen::Value::Number(value),
            Value::Text(value) => screen::Value::Text(value),
            Value::Numbers(values) => screen::Value::Numbers(values),
        };
        self.stage(Command::Value { name, value })
    }
}

impl cinnabar::server_experience::input::Host for State {
    /// True only for the declared, granted action this callback delivers.
    fn pressed(&mut self, action: String) -> Result<bool> {
        self.charge()?;
        Ok(self.action.as_ref() == Some(&action) && self.capabilities.may_deliver(&action))
    }
}

impl cinnabar::server_experience::messaging::Host for State {
    /// Parses and validates the signed channel record before queuing it.
    fn send(
        &mut self,
        channel: String,
        schema: u16,
        record_json: Vec<u8>,
    ) -> Result<Result<(), String>> {
        ensure!(record_json.len() <= MAX_PAYLOAD_BYTES, "message too large");
        let record = serde_json::from_slice(&record_json)?;
        self.stage(Command::Send {
            channel,
            schema,
            record,
        })
    }
}

impl cinnabar::server_experience::scene::Host for State {
    /// Accepts a bounded declarative object, never a native renderer handle.
    fn put(&mut self, id: u32, object_json: Option<Vec<u8>>) -> Result<Result<(), String>> {
        let object = object_json
            .map(|bytes| {
                ensure!(bytes.len() <= MAX_PAYLOAD_BYTES, "scene object too large");
                Ok::<_, anyhow::Error>(serde_json::from_slice(&bytes)?)
            })
            .transpose()?;
        self.stage(Command::Scene { id, object })
    }
}

impl cinnabar::server_experience::media::Host for State {
    /// Refers to an approved media descriptor, without exposing fetch APIs.
    fn control(
        &mut self,
        id: String,
        op: cinnabar::server_experience::media::Operation,
        position_ms: u64,
    ) -> Result<Result<(), String>> {
        use cinnabar::server_experience::media::Operation;
        let operation = match op {
            Operation::Prepare => MediaOperation::Prepare,
            Operation::Play => MediaOperation::Play,
            Operation::Pause => MediaOperation::Pause,
            Operation::Seek => MediaOperation::Seek,
            Operation::Stop => MediaOperation::Stop,
        };
        self.stage(Command::Media {
            id,
            operation,
            position_ms,
        })
    }
}

/// The guest's callbacks. A component built against 1.0 exports only `init` and `dispatch`;
/// 1.1 adds `action` and `epoch`, both or neither.
struct Exports {
    dispatch: Func,
    events: Option<(Func, Func)>,
}

impl Exports {
    /// Type-checks every callback once, before the guest runs.
    fn find(store: &mut Store<State>, instance: &Instance) -> Result<(Func, Self)> {
        let mut find = |name: &str| instance.get_func(&mut *store, name);
        let (init, dispatch, action, epoch) = (
            find("init"),
            find("dispatch"),
            find("action"),
            find("epoch"),
        );
        let (Some(init), Some(dispatch)) = (init, dispatch) else {
            anyhow::bail!("component lacks init or dispatch");
        };
        init.typed::<(), ()>(&*store)?;
        dispatch.typed::<(&str, &[u8]), ()>(&*store)?;
        let events = match (action, epoch) {
            (Some(action), Some(epoch)) => {
                action.typed::<(&str, Option<u32>), ()>(&*store)?;
                epoch.typed::<(), ()>(&*store)?;
                Some((action, epoch))
            }
            (None, None) => None,
            _ => anyhow::bail!("component exports only half of action and epoch"),
        };
        Ok((init, Self { dispatch, events }))
    }
}

pub struct BundleHost {
    store: Store<State>,
    exports: Exports,
    active: bool,
}

impl BundleHost {
    /// Unsafe for production remote code: this explicit developer path has no OS sandbox.
    pub fn developer_in_process(
        bytes: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self> {
        ensure!(
            std::env::var(DEVELOPER_ENV).as_deref() == Ok("1"),
            "in-process server code is developer-only"
        );
        Self::instantiate(bytes, owner, capabilities, epoch)
    }

    /// Compiles inside the helper, with no WASI or precompiled native cache input.
    pub(crate) fn instantiate(
        bytes: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self> {
        ensure!(
            bytes.len() <= MAX_COMPONENT_BYTES && bytes.starts_with(b"\0asm"),
            "invalid component bytes"
        );
        Self::launch(bytes, owner, capabilities, epoch)
    }

    /// Links the 1.1 imports, which also satisfy a 1.0 component's semver-compatible ones, and
    /// runs `init`.
    fn launch(
        source: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self> {
        capabilities.scope.validate()?;
        ensure!(
            capabilities.scope.memory_bytes > 0
                && capabilities.scope.memory_bytes <= MAX_GUEST_MEMORY,
            "invalid guest memory limit"
        );
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .consume_fuel(true)
            .max_wasm_stack(256 * 1024);
        let engine = Engine::new(&config)?;
        let component = Component::new(&engine, source)?;
        let mut linker = Linker::new(&engine);
        ServerBundle::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut State| state)?;
        let mut state = State {
            limits: StoreLimitsBuilder::new()
                .memory_size(capabilities.scope.memory_bytes as usize)
                .table_elements(4096)
                .instances(16)
                .memories(1)
                .tables(2)
                .trap_on_grow_failure(true)
                .build(),
            owner,
            epoch,
            capabilities,
            action: None,
            commands: Vec::new(),
            bytes: 0,
            calls: 0,
        };
        state.begin_output()?;
        let mut store = Store::new(&engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(CALLBACK_FUEL)?;
        let instance = linker.instantiate(&mut store, &component)?;
        let (init, exports) = Exports::find(&mut store, &instance)?;
        let init = init.typed::<(), ()>(&store)?;
        init.call(&mut store, ())?;
        init.post_return(&mut store)?;
        Ok(Self {
            store,
            exports,
            active: true,
        })
    }

    /// Runs one fuel-bounded callback; a trap discards all output and quarantines this instance.
    /// A 1.0 component skips `action` and `epoch` and returns an empty transaction.
    pub fn dispatch(&mut self, event: &Event, epoch: u64) -> Result<Transaction> {
        ensure!(self.active, "bundle quarantined");
        event.check()?;
        let state = self.store.data_mut();
        if let Event::Action { id, .. } = event {
            ensure!(state.capabilities.may_deliver(id), "action not granted");
        }
        state.action = match event {
            Event::Action { id, .. } => Some(id.clone()),
            _ => None,
        };
        state.epoch = epoch;
        state.begin_output()?;
        self.store.set_fuel(CALLBACK_FUEL)?;
        if let Err(error) = self.call(event) {
            self.active = false;
            self.store.data_mut().commands.clear();
            return Err(error);
        }
        Ok(self.take_transaction())
    }

    fn call(&mut self, event: &Event) -> Result<()> {
        let store = &mut self.store;
        match (event, &self.exports.events) {
            (Event::Message { channel, record }, _) => {
                let dispatch = self.exports.dispatch.typed::<(&str, &[u8]), ()>(&*store)?;
                dispatch.call(&mut *store, (channel, record))?;
                dispatch.post_return(store)
            }
            (Event::Action { id, index }, Some((action, _))) => {
                let action = action.typed::<(&str, Option<u32>), ()>(&*store)?;
                action.call(&mut *store, (id, *index))?;
                action.post_return(store)
            }
            (Event::Epoch, Some((_, epoch))) => {
                let epoch = epoch.typed::<(), ()>(&*store)?;
                epoch.call(&mut *store, ())?;
                epoch.post_return(store)
            }
            (Event::Action { .. } | Event::Epoch, None) => Ok(()),
        }
    }

    /// Takes only successfully returned initialization or callback output.
    pub fn take_transaction(&mut self) -> Transaction {
        let state = self.store.data_mut();
        Transaction {
            owner: state.owner.clone(),
            epoch: state.epoch,
            commands: std::mem::take(&mut state.commands),
        }
    }
}

#[cfg(test)]
mod tests;
