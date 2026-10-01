mod common;

use std::fs;
use std::path::Path;

use common::{edit_manifest, hello_wasm, probe_dir, probe_dir_with, probe_wasm, rehash};
use experience_runtime::limits::MAX_COMPONENT_BYTES;
use experience_runtime::load::{engine, load};
use experience_runtime::manifest::{ASSETS_DIR, SERVER_WASM};
use experience_runtime::protocol::{BlockDef, Mining, Texture};
use tempfile::TempDir;

/// Loads `dir`, which must fail, and returns the error chain. Every load error names the
/// directory.
fn refusal(dir: &Path) -> String {
    let (engine, _ticker) = engine().unwrap();
    let error = match load(&engine, dir) {
        Ok(_) => panic!("{} loaded", dir.display()),
        Err(error) => format!("{error:#}"),
    };
    let shown = dir.display().to_string();
    assert!(
        error.contains(&shown),
        "error does not name {shown}: {error}"
    );
    error
}

/// A probe artifact whose `server.wasm` is `bytes`, with the index following.
fn with_server_wasm(bytes: Vec<u8>) -> TempDir {
    probe_dir_with(|dir| {
        fs::write(dir.join(SERVER_WASM), bytes).unwrap();
        rehash(dir);
    })
}

/// The probe's core module with a custom section appended so it is exactly `len` bytes.
fn padded_probe(len: usize) -> Vec<u8> {
    const NAME: &[u8] = b"padding";
    let mut module = fs::read(probe_wasm()).unwrap();
    // Section id 0, its size as a 5-byte LEB128, the name; zeros fill the rest.
    let size = u32::try_from(len - module.len() - 6).unwrap();
    module.push(0);
    for shift in [0, 7, 14, 21] {
        module.push(0x80 | ((size >> shift) & 0x7f) as u8);
    }
    module.push((size >> 28) as u8);
    module.push(NAME.len() as u8);
    module.extend_from_slice(NAME);
    module.resize(len, 0);
    module
}

#[test]
fn probe_registers_counter_block() {
    let dir = probe_dir();
    let (engine, _ticker) = engine().unwrap();
    let loaded = load(&engine, dir.path()).unwrap();
    assert_eq!(loaded.manifest.id, "probe");
    assert_eq!(loaded.manifest.version, "0.1.0");
    let texture = dir.path().join(ASSETS_DIR).join("counter.png");
    assert_eq!(
        loaded.blocks,
        vec![BlockDef {
            id: "probe:counter".to_owned(),
            display_name: "Probe Counter".to_owned(),
            textures: vec![Texture {
                slot: "*".to_owned(),
                path: texture.to_str().unwrap().to_owned(),
            }],
            mining: Mining::Breakable { hardness: 1.0 },
        }]
    );
}

#[test]
fn foreign_namespace_is_refused() {
    let dir = probe_dir_with(|dir| {
        edit_manifest(dir, |manifest| {
            manifest.insert("id".to_owned(), "other".into());
        });
    });
    let error = refusal(dir.path());
    assert!(
        error.contains("probe:counter") && error.contains("namespace \"other:\""),
        "{error}"
    );
}

#[test]
fn hash_mismatch_is_refused() {
    let dir = probe_dir_with(|dir| {
        fs::write(dir.join(ASSETS_DIR).join("counter.png"), b"other bytes").unwrap();
    });
    let error = refusal(dir.path());
    let file = format!("{ASSETS_DIR}/counter.png");
    assert!(
        error.contains("hash mismatch") && error.contains(&file),
        "{error}"
    );
}

#[test]
fn unindexed_file_is_refused() {
    let dir = probe_dir_with(|dir| {
        fs::write(dir.join(ASSETS_DIR).join("unlisted.txt"), b"x").unwrap();
    });
    let error = refusal(dir.path());
    let unindexed = format!("unindexed file {ASSETS_DIR}/unlisted.txt");
    assert!(error.contains(&unindexed), "{error}");
}

#[test]
fn escaping_path_is_refused() {
    let escaping = [
        "../escape.txt",
        "assets/../../escape.txt",
        "/escape.txt",
        "C:/escape.txt",
        "..\\escape.txt",
    ];
    for key in escaping {
        let dir = probe_dir_with(|dir| {
            edit_manifest(dir, |manifest| {
                let files = manifest["files"].as_table_mut().unwrap();
                files.insert(key.to_owned(), "0".repeat(64).into());
            });
        });
        let error = refusal(dir.path());
        assert!(
            error.contains(&format!("invalid path \"{key}\"")),
            "{key}: {error}"
        );
    }
}

#[test]
fn wrong_api_is_refused() {
    let dir = probe_dir_with(|dir| {
        edit_manifest(dir, |manifest| {
            manifest.insert("api".to_owned(), "0.0".into());
        });
    });
    let error = refusal(dir.path());
    assert!(error.contains("unsupported api \"0.0\""), "{error}");
}

#[test]
fn core_module_without_world_is_refused() {
    let dir = with_server_wasm(wat::parse_str("(module)").unwrap());
    let error = refusal(dir.path());
    assert!(
        error.contains("is not a") && error.contains("server component"),
        "{error}"
    );
}

#[test]
fn client_component_is_refused() {
    let dir = with_server_wasm(fs::read(hello_wasm()).unwrap());
    let error = refusal(dir.path());
    assert!(
        error.contains("is not a") && error.contains("server component"),
        "{error}"
    );
}

/// The limit is inclusive: the probe padded to exactly `MAX_COMPONENT_BYTES` loads, and one
/// byte more is refused.
#[test]
fn oversized_component_is_refused() {
    let at_limit = with_server_wasm(padded_probe(MAX_COMPONENT_BYTES));
    let (engine, _ticker) = engine().unwrap();
    load(&engine, at_limit.path()).unwrap();

    let over = with_server_wasm(padded_probe(MAX_COMPONENT_BYTES + 1));
    let error = refusal(over.path());
    let limit = format!("{SERVER_WASM} exceeds {MAX_COMPONENT_BYTES} bytes");
    assert!(error.contains(&limit), "{error}");
}
