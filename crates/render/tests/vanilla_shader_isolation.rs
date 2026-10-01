//! Frozen vanilla shader bytes from b4354b66, after removing extension branches.
use sha2::{Digest, Sha256};

/// Removes only Enhanced preprocessor blocks, preserving every vanilla byte.
fn vanilla_source(source: &str) -> String {
    let mut active = vec![true];
    let mut result = String::new();
    for line in source.split_inclusive('\n') {
        let directive = line.trim();
        if directive.starts_with("#ifdef ENHANCED") {
            active.push(false);
        } else if directive == "#else" && active.len() > 1 {
            let last = active.last_mut().unwrap();
            *last = !*last;
        } else if directive == "#endif" && active.len() > 1 {
            active.pop();
        } else if active.iter().all(|value| *value) {
            result.push_str(line);
        }
    }
    assert_eq!(active.len(), 1);
    result
}

#[test]
fn disabled_enhanced_preserves_vanilla_shader_bytes() {
    for (source, digest) in [
        (
            include_str!("../src/chunk.wgsl"),
            "0af802af07c3c0c965ce12d62afea680c5f604d0ef3f5e460d2df08dfc5de448",
        ),
        (
            include_str!("../src/model.wgsl"),
            "431463ab75e1fb304d88cc045057990abe9b9a528afd1607e937d7275e4e5237",
        ),
        (
            include_str!("../src/liquid.wgsl"),
            "f99f53cff7f0d8a3077af0d9467223d30c64160b42d82327ccb98ce29e6b2358",
        ),
        (
            include_str!("../src/lighting.wgsl"),
            "88f6f8d5c0372ea94084d37a1ced7817aa1d1b3d2e733c6eeaec001181d25575",
        ),
        (
            include_str!("../src/biome_tint.wgsl"),
            "33a35a8404130304d72c7e26a34d8a62ded5b0208ae23054b8d0f1b980a3d25e",
        ),
        (
            include_str!("../src/atmosphere.wgsl"),
            "6bac2d0803b7e675efbbabbde767b4a3d2d78b4ac75cab1ec4a1ff99714dbd5a",
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(vanilla_source(source))),
            digest
        );
    }
}
