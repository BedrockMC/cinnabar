//! Experimental component host. Only the explicit WIT imports carry authority.

mod runtime;

use anyhow::{Context, Result, ensure};
use runtime::Instance;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use wasmtime::{Config, Engine};

/// Maximum bytes accepted before compilation or allocation of a package buffer.
pub const MAX_COMPONENT_BYTES: usize = 4 * 1024 * 1024;
/// Plain-text UI limit, checked before publishing any guest output.
pub const MAX_LABEL_BYTES: usize = 256;
pub(crate) const FRAME_FUEL: u64 = 100_000;
pub(crate) const MEMORY_BYTES: usize = 16 * 1024 * 1024;

/// A developer-selected component with transactional reload and trap quarantine.
pub struct ModHost {
    engine: Engine,
    instance: Instance,
    path: PathBuf,
    attempted: [u8; 32],
}

impl ModHost {
    /// Loads a local component, linking only HUD and the local demo action.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = read_component(path)?;
        let mut config = Config::new();
        config.wasm_component_model(true).consume_fuel(true);
        config.max_wasm_stack(256 * 1024);
        let engine = Engine::new(&config)?;
        let instance = Instance::new(&engine, &bytes)?;
        Ok(Self {
            engine,
            instance,
            path: path.to_owned(),
            attempted: Sha256::digest(&bytes).into(),
        })
    }

    /// Runs one bounded callback; a trap revokes its UI and disables the guest.
    pub fn frame(&mut self, pressed: bool) -> Result<()> {
        self.instance.frame(pressed)
    }

    /// Returns only the last successfully committed plain-text label.
    pub fn label(&self) -> Option<&str> {
        self.instance.label()
    }

    /// Whether this guest can still receive callbacks.
    pub fn is_active(&self) -> bool {
        self.instance.active
    }

    /// Replaces an instance only after changed bytes compile and initialize.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let bytes = read_component(&self.path)?;
        let digest = Sha256::digest(&bytes).into();
        if self.attempted == digest {
            return Ok(false);
        }
        self.attempted = digest;
        let candidate = Instance::new(&self.engine, &bytes)
            .context("reload rejected; previous mod retained")?;
        self.instance = candidate;
        Ok(true)
    }
}

/// Bounds file reads even if a writer grows the file between metadata and read.
fn read_component(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .with_context(|| format!("open mod {}", path.display()))?
        .take((MAX_COMPONENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_COMPONENT_BYTES,
        "component exceeds byte limit"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests;
