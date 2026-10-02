//! Frozen vanilla shader bytes from 471eaf88, after removing extension branches.
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
            "174e3339512fd062ca2bffc582f66193849f59261da645f738591b7c4ecb1554",
        ),
        (
            include_str!("../src/model.wgsl"),
            "f1b3b99c5d7eebe80599695f1a18a96e6638cd64dd0289c98006c7872ad0f85a",
        ),
        (
            include_str!("../src/liquid.wgsl"),
            "c81777dea15f56558aca67c1871985a7d44890b513be8a23413f1d1a2d62a5f1",
        ),
        (
            include_str!("../src/lighting.wgsl"),
            "ae7faeb7acea6a967a4e68d925c158b1db65053b2cf3003a1bbea5e61f4eae2c",
        ),
        (
            include_str!("../src/biome_tint.wgsl"),
            "cd04b8248192849c55dc46217710c5abfd5395ddef875a183140ed95ef4002a0",
        ),
        (
            include_str!("../src/atmosphere.wgsl"),
            "1bc2f8de37b6ac1fb6428586bca7f967b9884e1a175e138d99eb04024dae5feb",
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(vanilla_source(source))),
            digest
        );
    }
}

/// Freeze the real base descriptors as well as testing specialization mutations.
#[test]
fn vanilla_base_pipeline_construction_matches_baseline() {
    let source = include_str!("../src/chunk/pipeline/layouts.rs");
    let start = source
        .find("        let descriptor = RenderPipelineDescriptor")
        .unwrap();
    let end = source[start..]
        .find("\n#[derive(Clone, Copy, PartialEq")
        .unwrap()
        + start;
    let construction: String = source[start..end]
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect();
    assert_eq!(
        format!("{:x}", Sha256::digest(construction)),
        "29060b55955d474bb2e7d7361c76488275a706e196e477cfb851fd190541b838"
    );
}
