//! Loading a server artifact: verify it, compile it against the `server` world, run `register`
//! and validate the blocks it declares.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Engine};
use wit_component::ComponentEncoder;

use crate::hex;
use crate::host::cinnabar::experience_server::types as wit;
use crate::host::{self, HostState, Server, ServerPre};
use crate::limits::{
    EPOCH_PERIOD, MAX_BLOCKS, MAX_COMPONENT_BYTES, MAX_DISPLAY_NAME_BYTES, MAX_WASM_STACK_BYTES,
    REGISTER_DEADLINE, REGISTER_FUEL,
};
use crate::manifest::{ASSETS_DIR, Manifest, SERVER_WASM, read_manifest, resolve};
use crate::protocol;

/// The texture slots a block may bind, each at most once.
const SLOTS: [&str; 7] = ["*", "up", "down", "north", "south", "east", "west"];

/// A verified artifact: its manifest, its validated blocks, and the component pre-linked against
/// the `server` world, ready for a fresh instance per callback.
pub struct Loaded {
    pub manifest: Manifest,
    /// Texture paths are absolute.
    pub blocks: Vec<protocol::BlockDef>,
    #[expect(dead_code, reason = "callbacks instantiate from it; none run yet")]
    pub(crate) pre: ServerPre<HostState>,
}

/// Advances the engine's epoch once per elapsed [`EPOCH_PERIOD`]; dropping it stops and joins
/// the thread.
pub struct EpochTicker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl EpochTicker {
    fn start(engine: Engine) -> io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("epoch-ticker".to_owned())
            .spawn(move || {
                let start = Instant::now();
                let mut ticks = 0;
                while !stopped.load(Ordering::Relaxed) {
                    thread::sleep(EPOCH_PERIOD);
                    // Sleeps overshoot, so the epoch catches up with wall time instead of
                    // counting wake-ups.
                    let due = start.elapsed().as_nanos() / EPOCH_PERIOD.as_nanos();
                    for _ in ticks..due {
                        engine.increment_epoch();
                    }
                    ticks = due;
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The engine every Experience runs on: component model, fuel, epoch interruption and the wasm
/// stack limit, plus the ticker that drives its epoch.
pub fn engine() -> Result<(Engine, EpochTicker)> {
    let mut config = Config::new();
    config
        .wasm_component_model(true)
        .consume_fuel(true)
        .epoch_interruption(true)
        .max_wasm_stack(MAX_WASM_STACK_BYTES);
    let engine = Engine::new(&config)?;
    let ticker = EpochTicker::start(engine.clone()).context("starting the epoch ticker")?;
    Ok((engine, ticker))
}

/// Loads the artifact in `dir`. Errors name the directory and the cause.
pub fn load(engine: &Engine, dir: &Path) -> Result<Loaded> {
    load_dir(engine, dir).with_context(|| format!("loading experience {}", dir.display()))
}

fn load_dir(engine: &Engine, dir: &Path) -> Result<Loaded> {
    let manifest = read_manifest(dir)?;
    let module = read_module(dir, &manifest)?;
    let pre = link(engine, &module).with_context(|| {
        format!(
            "{SERVER_WASM} is not a {} server component",
            host::wit_package()
        )
    })?;
    let defs = register(engine, &pre, &manifest.id)?;
    let blocks = validate_blocks(dir, &manifest, defs)?;
    Ok(Loaded {
        manifest,
        blocks,
        pre,
    })
}

/// Reads `server.wasm`, refusing more than [`MAX_COMPONENT_BYTES`], and checks that the bytes it
/// will compile are the indexed ones.
fn read_module(dir: &Path, manifest: &Manifest) -> Result<Vec<u8>> {
    let mut module = Vec::new();
    File::open(dir.join(SERVER_WASM))
        .and_then(|file| {
            file.take(MAX_COMPONENT_BYTES as u64 + 1)
                .read_to_end(&mut module)
        })
        .with_context(|| format!("reading {SERVER_WASM}"))?;
    ensure!(
        module.len() <= MAX_COMPONENT_BYTES,
        "{SERVER_WASM} exceeds {MAX_COMPONENT_BYTES} bytes"
    );
    let indexed = manifest
        .files
        .get(SERVER_WASM)
        .with_context(|| format!("{SERVER_WASM} is not indexed"))?;
    ensure!(
        hex::encode(&Sha256::digest(&module)) == *indexed,
        "{SERVER_WASM} changed after its hash was verified"
    );
    Ok(module)
}

/// Turns the core module into a component and links it against exactly the `server` world's
/// imports; a client world, a WASI import or a missing export fails here.
fn link(engine: &Engine, module: &[u8]) -> Result<ServerPre<HostState>> {
    let component = ComponentEncoder::default()
        .module(module)?
        .validate(true)
        .encode()?;
    let component = Component::new(engine, &component)?;
    let mut linker = Linker::new(engine);
    Server::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)?;
    ServerPre::new(linker.instantiate_pre(&component)?)
}

/// Runs `register` once on a fresh instance under the register fuel and deadline.
fn register(engine: &Engine, pre: &ServerPre<HostState>, id: &str) -> Result<Vec<wit::BlockDef>> {
    let mut store = HostState::store(engine, id, REGISTER_FUEL, REGISTER_DEADLINE)?;
    let server = pre
        .instantiate(&mut store)
        .with_context(|| format!("instantiating {SERVER_WASM}"))?;
    match server
        .call_register(&mut store)
        .context("register trapped")?
    {
        Ok(defs) => Ok(defs),
        Err(wit::GuestError::Rejected(reason) | wit::GuestError::Failed(reason)) => {
            bail!("register failed: {reason}")
        }
    }
}

/// Checks every declared block; texture paths become absolute under the artifact directory.
fn validate_blocks(
    dir: &Path,
    manifest: &Manifest,
    defs: Vec<wit::BlockDef>,
) -> Result<Vec<protocol::BlockDef>> {
    ensure!(
        defs.len() <= MAX_BLOCKS,
        "register declared {} blocks; the limit is {MAX_BLOCKS}",
        defs.len()
    );
    let root = std::path::absolute(dir).context("resolving the artifact directory")?;
    let namespace = format!("{}:", manifest.id);
    let mut blocks: Vec<protocol::BlockDef> = Vec::with_capacity(defs.len());
    for def in defs {
        let wit::BlockDef {
            id,
            display_name,
            textures,
            mining,
        } = def;
        ensure!(
            id.starts_with(&namespace),
            "block \"{id}\" is outside namespace \"{namespace}\""
        );
        ensure!(
            blocks.iter().all(|block| block.id != id),
            "block \"{id}\" is declared twice"
        );
        let (textures, mining) =
            validate_block(&root, &manifest.files, &display_name, textures, mining)
                .with_context(|| format!("block \"{id}\""))?;
        blocks.push(protocol::BlockDef {
            id,
            display_name,
            textures,
            mining,
        });
    }
    Ok(blocks)
}

/// Checks one block's display name, texture bindings and mining.
fn validate_block(
    root: &Path,
    files: &BTreeMap<String, String>,
    display_name: &str,
    bindings: Vec<wit::TextureBinding>,
    mining: wit::Mining,
) -> Result<(Vec<protocol::Texture>, protocol::Mining)> {
    ensure!(
        (1..=MAX_DISPLAY_NAME_BYTES).contains(&display_name.len()),
        "display name has {} bytes; it needs 1 to {MAX_DISPLAY_NAME_BYTES}",
        display_name.len()
    );
    ensure!(
        !display_name.chars().any(char::is_control),
        "display name {display_name:?} has a control character"
    );
    let mut textures: Vec<protocol::Texture> = Vec::new();
    for wit::TextureBinding { slot, path } in bindings {
        ensure!(
            SLOTS.contains(&slot.as_str()),
            "texture slot \"{slot}\" is not one of {SLOTS:?}"
        );
        ensure!(
            textures.iter().all(|texture| texture.slot != slot),
            "texture slot \"{slot}\" is bound twice"
        );
        // Index keys are already confined to the artifact, so a listed key is safe to join.
        let indexed = format!("{ASSETS_DIR}/{path}");
        ensure!(
            files.contains_key(&indexed),
            "texture \"{path}\" is not an indexed file: [files] has no \"{indexed}\""
        );
        let path = resolve(root, &indexed)
            .into_os_string()
            .into_string()
            .map_err(|path| anyhow!("{} is not a UTF-8 path", path.display()))?;
        textures.push(protocol::Texture { slot, path });
    }
    let mining = match mining {
        wit::Mining::Unbreakable => protocol::Mining::Unbreakable {},
        wit::Mining::Breakable(hardness) => {
            ensure!(
                hardness.is_finite() && hardness >= 0.0,
                "hardness {hardness} is not a finite number ≥ 0"
            );
            protocol::Mining::Breakable { hardness }
        }
    };
    Ok((textures, mining))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use wasmtime::Trap;
    use wasmtime::component::{Component, Linker};

    use super::engine;
    use crate::host::HostState;

    /// The ticker keeps the epoch on wall time, so a store with fuel to spare still stops at its
    /// deadline, and not long before or after it.
    #[test]
    fn epoch_deadline_stops_a_store_with_fuel_left() {
        const DEADLINE: Duration = Duration::from_millis(200);
        // Seconds of spinning: a stalled epoch ends in a fuel trap instead of a hang.
        const FUEL: u64 = 10_000_000_000;
        let (engine, _ticker) = engine().unwrap();
        let spin = r#"(component
            (core module $m (func (export "spin") (loop (br 0))))
            (core instance $i (instantiate $m))
            (func (export "spin") (canon lift (core func $i "spin"))))"#;
        let component = Component::new(&engine, spin).unwrap();
        let mut store = HostState::store(&engine, "spin", FUEL, DEADLINE).unwrap();
        let instance = Linker::new(&engine)
            .instantiate(&mut store, &component)
            .unwrap();
        let spin = instance
            .get_typed_func::<(), ()>(&mut store, "spin")
            .unwrap();
        let start = Instant::now();
        let error = spin.call(&mut store, ()).unwrap_err();
        let elapsed = start.elapsed();
        assert_eq!(
            error.downcast_ref::<Trap>(),
            Some(&Trap::Interrupt),
            "{error:#}"
        );
        assert!(
            elapsed >= DEADLINE / 2 && elapsed < DEADLINE * 5,
            "stopped after {elapsed:?}"
        );
    }
}
