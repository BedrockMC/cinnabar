use crate::{CameraDelta, FRAME_FUEL, GameplaySnapshot, MAX_LABEL_BYTES, MEMORY_BYTES, ModGrants};
use anyhow::{Result, bail};
use wasmtime::{
    Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, HasSelf, Linker},
};

wasmtime::component::bindgen!({
    path: "../mod-api/wit", world: "extension", imports: { default: trappable },
});

const MAX_IMPORT_WRITES: u32 = 8;
#[path = "controls.rs"]
mod controls;
#[path = "gameplay.rs"]
mod gameplay;
#[path = "render.rs"]
mod render;

struct State {
    limits: StoreLimits,
    pressed: bool,
    label: Option<String>,
    pending: Option<String>,
    writes: u32,
    grants: ModGrants,
    time_override: Option<u32>,
    pending_time: Option<Option<u32>>,
    environment_writes: u32,
    snapshot: Option<GameplaySnapshot>,
    gameplay_reads: u32,
    camera_writes: u32,
    pending_camera: Option<CameraDelta>,
    camera_delta: Option<CameraDelta>,
    controls: controls::ControlState,
    render: render::RenderState,
}

impl cinnabar::extension::hud::Host for State {
    /// Stages bounded plain text; nothing is published until the guest returns.
    fn set_label(&mut self, text: String) -> Result<Result<(), String>> {
        self.writes += 1;
        if self.writes > MAX_IMPORT_WRITES {
            bail!("HUD import budget exhausted");
        }
        if text.len() > MAX_LABEL_BYTES || text.chars().any(|c| c.is_control() || c == '§') {
            return Ok(Err("label must be short plain text".into()));
        }
        self.pending = Some(text);
        Ok(Ok(()))
    }
}

impl cinnabar::extension::environment::Host for State {
    /// Stages a fixed visual clock only with explicit authority and a valid tick.
    fn set_time_override(&mut self, ticks: Option<u32>) -> Result<Result<(), String>> {
        self.environment_writes += 1;
        if self.environment_writes > MAX_IMPORT_WRITES {
            bail!("environment import budget exhausted");
        }
        if !self.grants.environment {
            return Ok(Err("environment capability denied".into()));
        }
        if ticks.is_some_and(|tick| tick >= mod_api::BEDROCK_DAY_TICKS) {
            return Ok(Err("time override must be within one Bedrock day".into()));
        }
        self.pending_time = Some(ticks);
        Ok(Ok(()))
    }
}

impl cinnabar::extension::input::Host for State {
    /// Reads only the host's one-frame action edge, never raw keyboard state.
    fn demo_pressed(&mut self) -> Result<bool> {
        Ok(self.pressed)
    }

    fn read_controls(&mut self) -> Result<Result<crate::ControlFrame, String>> {
        controls::read(self)
    }

    fn reserve_keys(&mut self, keys: Vec<String>) -> Result<Result<(), String>> {
        controls::reserve(self, keys)
    }
}

pub(super) struct Instance {
    store: Store<State>,
    guest: Extension,
    pub(super) active: bool,
}

impl Instance {
    /// Initializes a candidate store without changing the published instance.
    pub(super) fn new(
        engine: &Engine,
        bytes: &[u8],
        grants: ModGrants,
        settings: String,
    ) -> Result<Self> {
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
            grants,
            time_override: None,
            pending_time: None,
            environment_writes: 0,
            snapshot: None,
            gameplay_reads: 0,
            camera_writes: 0,
            pending_camera: None,
            camera_delta: None,
            controls: controls::ControlState::new(settings),
            render: render::RenderState::new(),
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
    pub(super) fn frame(
        &mut self,
        pressed: bool,
        snapshot: Option<GameplaySnapshot>,
        controls: crate::ControlFrame,
    ) -> Result<()> {
        let state = self.store.data_mut();
        state.snapshot = None;
        state.pending_camera = None;
        state.camera_delta = None;
        state.controls.begin_frame();
        state.render.begin_frame();
        if !self.active {
            return Ok(());
        }
        gameplay::validate_snapshot(snapshot.as_ref())?;
        controls::validate_frame(&controls)?;
        let state = self.store.data_mut();
        state.pressed = pressed;
        state.writes = 0;
        state.environment_writes = 0;
        state.gameplay_reads = 0;
        state.camera_writes = 0;
        state.snapshot = snapshot;
        state.controls.frame = controls;
        self.store.set_fuel(FRAME_FUEL)?;
        if let Err(error) = self.guest.call_frame(&mut self.store) {
            self.active = false;
            self.store.data_mut().pending = None;
            self.store.data_mut().label = None;
            self.store.data_mut().pending_time = None;
            self.store.data_mut().time_override = None;
            self.store.data_mut().snapshot = None;
            self.store.data_mut().pending_camera = None;
            self.store.data_mut().camera_delta = None;
            self.store.data_mut().controls.revoke();
            self.store.data_mut().render.revoke();
            bail!("mod quarantined after a guest trap: {error:#}");
        }
        commit(&mut self.store);
        self.store.data_mut().snapshot = None;
        self.store.data_mut().controls.frame = crate::empty_controls();
        Ok(())
    }

    pub(super) fn take_camera_delta(&mut self) -> Option<CameraDelta> {
        self.store.data_mut().camera_delta.take()
    }

    /// Reads the committed presentation clock without entering the component.
    pub(super) fn time_override(&self) -> Option<u32> {
        self.store.data().time_override
    }

    /// Reads retained UI without entering the component.
    pub(super) fn label(&self) -> Option<&str> {
        self.store.data().label.as_deref()
    }

    pub(super) fn panel(&self) -> Option<&ui::mod_panel::Panel> {
        self.store.data().controls.panel.as_ref()
    }
    pub(super) fn panel_open(&self) -> bool {
        self.store.data().controls.open
    }
    pub(super) fn set_panel_open(&mut self, open: bool) {
        let state = self.store.data_mut();
        state.controls.open =
            open && self.active && state.grants.controls && state.controls.panel.is_some();
    }
    pub(super) fn reserved_keys(&self) -> &[String] {
        &self.store.data().controls.keys
    }
    pub(super) fn take_interaction(&mut self) -> crate::InteractionOutput {
        std::mem::take(&mut self.store.data_mut().controls.interaction)
    }
    pub(super) fn settings_write(&self) -> Option<&str> {
        self.store.data().controls.dirty_settings.as_deref()
    }
    pub(super) fn settings_written(&mut self) {
        self.store.data_mut().controls.dirty_settings = None;
    }

    pub(super) fn render(&self) -> (&mod_render::RenderOutput, u64) {
        let render = &self.store.data().render;
        (render.output(), render.generation())
    }

    pub(super) fn settings(&self) -> &str {
        self.store.data().controls.settings()
    }
}

/// Publishes retained presentation changes after the entire callback succeeds.
fn commit(store: &mut Store<State>) {
    let state = store.data_mut();
    state.controls.commit();
    state.render.commit();
    state.camera_delta = state.pending_camera.take();
    if let Some(ticks) = state.pending_time.take() {
        state.time_override = ticks;
    }
    if let Some(text) = state.pending.take() {
        state.label = (!text.is_empty()).then_some(text);
    }
}
