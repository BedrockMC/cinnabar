use crate::{FRAME_FUEL, MAX_LABEL_BYTES, MEMORY_BYTES};
use anyhow::{Result, bail};
use wasmtime::{
    Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, HasSelf, Linker},
};

wasmtime::component::bindgen!({
    path: "../mod-api/wit", world: "extension", imports: { default: trappable },
});

struct State {
    limits: StoreLimits,
    pressed: bool,
    label: Option<String>,
    pending: Option<String>,
    writes: u32,
}

impl cinnabar::extension::hud::Host for State {
    /// Stages bounded plain text; nothing is published until the guest returns.
    fn set_label(&mut self, text: String) -> Result<Result<(), String>> {
        self.writes += 1;
        if self.writes > 8 {
            bail!("HUD import budget exhausted");
        }
        if text.len() > MAX_LABEL_BYTES || text.chars().any(|c| c.is_control() || c == '§') {
            return Ok(Err("label must be short plain text".into()));
        }
        self.pending = Some(text);
        Ok(Ok(()))
    }
}

impl cinnabar::extension::input::Host for State {
    /// Reads only the host's one-frame action edge, never raw keyboard state.
    fn demo_pressed(&mut self) -> Result<bool> {
        Ok(self.pressed)
    }
}

pub(super) struct Instance {
    store: Store<State>,
    guest: Extension,
    pub(super) active: bool,
}

impl Instance {
    /// Initializes a candidate store without changing the published instance.
    pub(super) fn new(engine: &Engine, bytes: &[u8]) -> Result<Self> {
        let component = Component::new(engine, bytes)?;
        let mut linker = Linker::new(engine);
        Extension::add_to_linker::<_, HasSelf<_>>(&mut linker, |state: &mut State| state)?;
        let state = State {
            limits: StoreLimitsBuilder::new()
                .memory_size(MEMORY_BYTES)
                .table_elements(4096)
                .instances(16)
                .memories(1)
                .tables(2)
                .trap_on_grow_failure(true)
                .build(),
            pressed: false,
            label: None,
            pending: None,
            writes: 0,
        };
        let mut store = Store::new(engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_fuel(FRAME_FUEL)?;
        let guest = Extension::instantiate(&mut store, &component, &linker)?;
        guest.call_init(&mut store)?;
        commit(&mut store);
        Ok(Self {
            store,
            guest,
            active: true,
        })
    }

    /// Restores the call budget and commits output only on successful return.
    pub(super) fn frame(&mut self, pressed: bool) -> Result<()> {
        if !self.active {
            return Ok(());
        }
        let state = self.store.data_mut();
        state.pressed = pressed;
        state.writes = 0;
        self.store.set_fuel(FRAME_FUEL)?;
        if let Err(error) = self.guest.call_frame(&mut self.store) {
            self.active = false;
            self.store.data_mut().pending = None;
            self.store.data_mut().label = None;
            bail!("mod quarantined after a guest trap: {error:#}");
        }
        commit(&mut self.store);
        Ok(())
    }

    /// Reads retained UI without entering the component.
    pub(super) fn label(&self) -> Option<&str> {
        self.store.data().label.as_deref()
    }
}

/// Publishes at most one label mutation after the entire callback succeeds.
fn commit(store: &mut Store<State>) {
    let state = store.data_mut();
    if let Some(text) = state.pending.take() {
        state.label = (!text.is_empty()).then_some(text);
    }
}
