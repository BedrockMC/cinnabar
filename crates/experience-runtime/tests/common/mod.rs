//! Builds guest crates for wasm32 and assembles them into temporary server artifacts, and builds
//! the probe's callback requests. The probe selects a behavior by the interacted block's x; `p(x)`
//! is that block and `up(x)` the one above.
#![allow(dead_code, reason = "each test binary uses only some of these helpers")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use experience_runtime::callback::run;
use experience_runtime::load::{EpochTicker, Loaded, engine, load};
use experience_runtime::manifest::{ASSETS_DIR, MANIFEST_FILE, SERVER_WASM};
use experience_runtime::protocol::{BlockPos, Call, Cell, Face, Info, Op, Outcome, Request};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wasmtime::Engine;

pub const ACTOR: &str = "3f2a7c1e-8b4d-4e6a-9c5f-1d2e3f4a5b6c";
pub const COUNTER: &str = "probe:counter";
pub const AIR: &str = "minecraft:air";

const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// The probe's `assets/counter.png`: a 1×1 opaque RGBA PNG.
const COUNTER_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x68, 0x68, 0x68, 0xf8,
    0x0f, 0x00, 0x05, 0x84, 0x02, 0x80, 0x53, 0x93, 0x74, 0x36, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the crate lives two levels below the workspace root")
}

/// The Cargo target directory of this test binary, `<target>/<profile>/deps/<test>`.
fn target_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("test binary path");
    exe.ancestors()
        .nth(3)
        .expect("test binary lives in <target>/<profile>/deps")
        .to_owned()
}

/// Builds `package` for wasm32 and returns its `.wasm`. The nested Cargo uses a target directory
/// of its own so it never waits on the build lock held by the running `cargo test`.
fn build_guest(package: &str) -> PathBuf {
    let target = target_dir().join("experience-guests");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .current_dir(workspace_root())
        .args(["build", "--locked", "--target", WASM_TARGET, "-p", package])
        .arg("--target-dir")
        .arg(&target)
        .output()
        .unwrap_or_else(|e| panic!("running cargo for {package}: {e}"));
    assert!(
        output.status.success(),
        "building {package} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let file = format!("{}.wasm", package.replace('-', "_"));
    target.join(WASM_TARGET).join("debug").join(file)
}

/// The probe guest's core module, built once per test binary.
pub fn probe_wasm() -> &'static Path {
    static WASM: LazyLock<PathBuf> = LazyLock::new(|| build_guest("experience-probe"));
    &WASM
}

/// The client `hello-mod` guest's core module, built once per test binary.
pub fn hello_wasm() -> &'static Path {
    static WASM: LazyLock<PathBuf> = LazyLock::new(|| build_guest("hello-mod"));
    &WASM
}

/// A fresh probe artifact: the probe's `experience.toml` with real hashes, `server.wasm` and
/// `assets/counter.png`. It is deleted when the returned guard drops.
pub fn probe_dir() -> TempDir {
    probe_dir_with(|_| {})
}

/// A fresh probe artifact after `edit` ran on it. `edit` calls [`rehash`] when the index should
/// follow its changes.
pub fn probe_dir_with(edit: impl FnOnce(&Path)) -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let root = dir.path();
    let probe = workspace_root().join("examples/experiences/probe");
    fs::copy(probe.join(MANIFEST_FILE), root.join(MANIFEST_FILE)).expect("copy manifest");
    fs::copy(probe_wasm(), root.join(SERVER_WASM)).expect("copy server.wasm");
    fs::create_dir(root.join(ASSETS_DIR)).expect("create assets/");
    fs::write(root.join(ASSETS_DIR).join("counter.png"), COUNTER_PNG).expect("write counter.png");
    rehash(root);
    edit(root);
    dir
}

/// Rewrites `[files]` with the SHA-256 of every file except the manifest.
pub fn rehash(dir: &Path) {
    let files: toml::Table = relative_files(dir)
        .into_iter()
        .filter(|path| path != MANIFEST_FILE)
        .map(|path| {
            let hash = format!("{:x}", Sha256::digest(fs::read(dir.join(&path)).unwrap()));
            (path, toml::Value::String(hash))
        })
        .collect();
    edit_manifest(dir, |manifest| {
        manifest.insert("files".to_owned(), toml::Value::Table(files));
    });
}

/// Applies `edit` to the parsed `experience.toml` and writes it back.
pub fn edit_manifest(dir: &Path, edit: impl FnOnce(&mut toml::Table)) {
    let path = dir.join(MANIFEST_FILE);
    let mut manifest: toml::Table = fs::read_to_string(&path)
        .expect("read manifest")
        .parse()
        .expect("parse manifest");
    edit(&mut manifest);
    fs::write(&path, toml::to_string(&manifest).unwrap()).expect("write manifest");
}

/// Every file below `dir`, as a `/`-separated path relative to it.
fn relative_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if entry.file_type().unwrap().is_dir() {
            let nested = relative_files(&entry.path());
            files.extend(nested.into_iter().map(|path| format!("{name}/{path}")));
        } else {
            files.push(name);
        }
    }
    files
}

/// The probe, loaded once per test binary.
pub struct Probe {
    pub engine: Engine,
    pub loaded: Loaded,
    _ticker: EpochTicker,
}

pub fn probe() -> &'static Probe {
    static PROBE: LazyLock<Probe> = LazyLock::new(|| {
        let (engine, ticker) = engine().unwrap();
        // The artifact is only read while loading.
        let dir = probe_dir();
        let loaded = load(&engine, dir.path()).unwrap();
        Probe {
            engine,
            loaded,
            _ticker: ticker,
        }
    });
    &PROBE
}

/// Runs `request` on the probe.
pub fn outcome(request: &Request) -> Outcome {
    let probe = probe();
    run(&probe.engine, &probe.loaded, request)
}

pub fn p(x: i32) -> BlockPos {
    BlockPos { x, y: 64, z: 0 }
}

pub fn up(x: i32) -> BlockPos {
    BlockPos { x, y: 65, z: 0 }
}

/// A loaded cell; `data` is hex.
pub fn cell(pos: BlockPos, id: &str, owned: bool, data: Option<&str>) -> Cell {
    Cell {
        pos,
        loaded: true,
        id: id.to_owned(),
        owned,
        data: data.map(str::to_owned),
    }
}

/// A callback from the actor for `call` at `anchor`, with a 7-cell snapshot: the anchor is an
/// owned probe:counter without data and its six neighbors are loaded air. The world height and the
/// data budget leave room.
pub fn callback(anchor: BlockPos, call: Call) -> Request {
    let BlockPos { x, y, z } = anchor;
    let neighbors = [
        (x + 1, y, z),
        (x - 1, y, z),
        (x, y + 1, z),
        (x, y - 1, z),
        (x, y, z + 1),
        (x, y, z - 1),
    ];
    let mut snapshot = vec![cell(anchor, COUNTER, true, None)];
    snapshot.extend(
        neighbors
            .into_iter()
            .map(|(x, y, z)| cell(BlockPos { x, y, z }, AIR, false, None)),
    );
    Request::Callback {
        seq: 1,
        info: Info {
            world_id: "world".to_owned(),
            dimension_id: "overworld".to_owned(),
            tick: 1,
            event_sequence: 1,
        },
        actor: Some(ACTOR.to_owned()),
        world_min_y: -64,
        world_max_y: 319,
        data_budget: 1 << 20,
        snapshot,
        call,
    }
}

/// The actor's interaction with `p(x)`.
pub fn interact(x: i32) -> Request {
    callback(
        p(x),
        Call::Interact {
            player: ACTOR.to_owned(),
            pos: p(x),
            face: Face::Up,
        },
    )
}

/// A tell to the actor.
pub fn tell(text: &str) -> Op {
    Op::Tell {
        player: ACTOR.to_owned(),
        text: text.to_owned(),
    }
}
