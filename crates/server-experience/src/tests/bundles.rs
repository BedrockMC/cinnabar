use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::*;

/// Builds a real signed archive with one deliberately tiny portable payload.
fn archive(path: &str, content: &[u8], extra: bool) -> (Vec<u8>, manifest::Offer) {
    archive_files(&[(path, content)], extra, CompressionMethod::Stored)
}

/// Signs all assets independently so forged ZIP sizes cannot change the trusted index.
fn archive_files(
    assets: &[(&str, &[u8])],
    extra: bool,
    method: CompressionMethod,
) -> (Vec<u8>, manifest::Offer) {
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let mut deployment = offer(&key);
    let manifest = manifest::Manifest {
        version: policy::WIRE_VERSION,
        api: policy::API_VERSION,
        id: deployment.packages[0].id.clone(),
        publisher_key: deployment.packages[0].publisher_key.clone(),
        package_version: "fixture".into(),
        permissions: BTreeSet::new(),
        component: None,
        channels: Vec::new(),
        actions: BTreeSet::new(),
        files: assets
            .iter()
            .map(|(path, content)| manifest::ContentFile {
                path: (*path).into(),
                bytes: content.len() as u64,
                sha256: crypto::digest(content),
            })
            .collect(),
    };
    let document = serde_json::to_vec(&signed(&manifest, crypto::MANIFEST_DOMAIN, &key)).unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(method);
    writer.start_file(bundle::MANIFEST_PATH, options).unwrap();
    writer.write_all(&document).unwrap();
    for (path, content) in assets {
        writer.start_file(*path, options).unwrap();
        writer.write_all(content).unwrap();
    }
    if extra {
        writer.start_file("unindexed.txt", options).unwrap();
        writer.write_all(b"hidden").unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    deployment.packages[0].bytes = bytes.len() as u64;
    deployment.packages[0].digest = crypto::digest(&bytes);
    (bytes, deployment)
}

#[test]
fn indexed_archive_is_verified_before_any_runtime_exists() {
    let (bytes, deployment) = archive("poster.txt", b"fixture", false);
    let bundle = bundle::VerifiedBundle::read(
        &bytes,
        &deployment.packages[0],
        &deployment.scope,
        policy::MAX_EXPANDED_BYTES,
    )
    .unwrap();
    assert_eq!(bundle.file("poster.txt"), Some(b"fixture".as_slice()));
    assert!(bundle.component().is_none());
    assert_eq!(bundle.expanded_bytes(), 7);
    assert!(
        bundle::VerifiedBundle::read(&bytes, &deployment.packages[0], &deployment.scope, 1,)
            .is_err()
    );
}

#[test]
fn signed_archives_still_reject_unindexed_and_unsafe_entries() {
    for (path, extra) in [("poster.txt", true), ("../escape.txt", false)] {
        let (bytes, deployment) = archive(path, b"fixture", extra);
        assert!(
            bundle::VerifiedBundle::read(
                &bytes,
                &deployment.packages[0],
                &deployment.scope,
                policy::MAX_EXPANDED_BYTES,
            )
            .is_err()
        );
    }
}

#[test]
fn outer_integrity_and_publisher_pins_are_independent_checks() {
    let (mut bytes, mut deployment) = archive("poster.txt", b"fixture", false);
    deployment.packages[0].publisher_key = crypto::hex(&[1; 32]);
    assert!(
        bundle::VerifiedBundle::read(
            &bytes,
            &deployment.packages[0],
            &deployment.scope,
            policy::MAX_EXPANDED_BYTES,
        )
        .is_err()
    );
    bytes[0] ^= 1;
    assert!(
        bundle::VerifiedBundle::read(
            &bytes,
            &deployment.packages[0],
            &deployment.scope,
            policy::MAX_EXPANDED_BYTES,
        )
        .is_err()
    );
}

/// Updates only the outer digest after corrupting a directory; signed asset sizes stay unchanged.
fn read_mutated(
    bytes: &[u8],
    deployment: &mut manifest::Offer,
    limit: u64,
) -> anyhow::Result<bundle::VerifiedBundle> {
    deployment.packages[0].bytes = bytes.len() as u64;
    deployment.packages[0].digest = crypto::digest(bytes);
    bundle::VerifiedBundle::read(bytes, &deployment.packages[0], &deployment.scope, limit)
}

/// Finds central entries in these fixtures, whose contents do not contain ZIP signatures.
fn central_entries(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(4)
        .enumerate()
        .filter_map(|(i, bytes)| (bytes == b"PK\x01\x02").then_some(i))
        .collect()
}

#[test]
fn forged_deflate_sizes_cannot_bypass_the_remaining_multi_asset_budget() {
    let asset = vec![b'x'; 4096];
    let (mut bytes, mut deployment) = archive_files(
        &[("first.bin", &asset), ("other.bin", &asset)],
        false,
        CompressionMethod::Deflated,
    );
    let entries = central_entries(&bytes);
    let manifest_size =
        u32::from_le_bytes(bytes[entries[0] + 24..entries[0] + 28].try_into().unwrap()) as u64;
    assert!(read_mutated(&bytes, &mut deployment, manifest_size + 8191).is_err());
    assert!(read_mutated(&bytes, &mut deployment, manifest_size + 8192).is_ok());
    for at in entries.into_iter().skip(1) {
        bytes[at + 24..at + 28].copy_from_slice(&1u32.to_le_bytes());
    }
    let error = read_mutated(&bytes, &mut deployment, manifest_size + 2).unwrap_err();
    assert!(error.to_string().contains("signed size"), "{error}");
}

#[test]
fn physical_duplicates_are_rejected_before_zip_deduplicates_them() {
    let (mut bytes, mut deployment) = archive_files(
        &[("first.bin", b"a"), ("other.bin", b"b")],
        false,
        CompressionMethod::Stored,
    );
    let at = central_entries(&bytes)[2];
    bytes[at + 46..at + 55].copy_from_slice(b"first.bin");
    assert!(
        read_mutated(&bytes, &mut deployment, policy::MAX_EXPANDED_BYTES)
            .unwrap_err()
            .to_string()
            .contains("duplicate physical")
    );
}

#[test]
fn physical_entry_count_is_bounded_before_library_indexing() {
    let (bytes, mut deployment) = archive("first.bin", b"a", false);
    let entries = central_entries(&bytes);
    let end = bytes.len() - 22;
    let entry = &bytes[entries[1]..end];
    let mut many = bytes[..entries[0]].to_vec();
    for _ in 0..policy::MAX_FILES + 2 {
        many.extend_from_slice(entry);
    }
    let size = many.len() - entries[0];
    let mut footer = bytes[end..].to_vec();
    let count = (policy::MAX_FILES + 2) as u16;
    footer[8..10].copy_from_slice(&count.to_le_bytes());
    footer[10..12].copy_from_slice(&count.to_le_bytes());
    footer[12..16].copy_from_slice(&(size as u32).to_le_bytes());
    many.extend_from_slice(&footer);
    assert!(
        read_mutated(&many, &mut deployment, policy::MAX_EXPANDED_BYTES)
            .unwrap_err()
            .to_string()
            .contains("too many physical")
    );
    let end = many.len() - 22;
    many[end + 8..end + 10].copy_from_slice(&1u16.to_le_bytes());
    many[end + 10..end + 12].copy_from_slice(&1u16.to_le_bytes());
    assert!(
        read_mutated(&many, &mut deployment, policy::MAX_EXPANDED_BYTES)
            .unwrap_err()
            .to_string()
            .contains("unaccounted physical")
    );
}
