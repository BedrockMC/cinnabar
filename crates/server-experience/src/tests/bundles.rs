use std::io::{Cursor, Write};

use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::*;

/// Builds a real signed archive with one deliberately tiny portable payload.
fn archive(path: &str, content: &[u8], extra: bool) -> (Vec<u8>, manifest::Offer) {
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
        files: vec![manifest::ContentFile {
            path: path.into(),
            bytes: content.len() as u64,
            sha256: crypto::digest(content),
        }],
    };
    let document = serde_json::to_vec(&signed(&manifest, crypto::MANIFEST_DOMAIN, &key))
        .unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file(bundle::MANIFEST_PATH, options).unwrap();
    writer.write_all(&document).unwrap();
    writer.start_file(path, options).unwrap();
    writer.write_all(content).unwrap();
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
    ).unwrap();
    assert_eq!(bundle.file("poster.txt"), Some(b"fixture".as_slice()));
    assert!(bundle.component().is_none());
    assert_eq!(bundle.expanded_bytes(), 7);
    assert!(bundle::VerifiedBundle::read(
        &bytes, &deployment.packages[0], &deployment.scope, 1,
    ).is_err());
}

#[test]
fn signed_archives_still_reject_unindexed_and_unsafe_entries() {
    for (path, extra) in [("poster.txt", true), ("../escape.txt", false)] {
        let (bytes, deployment) = archive(path, b"fixture", extra);
        assert!(bundle::VerifiedBundle::read(
            &bytes,
            &deployment.packages[0],
            &deployment.scope,
            policy::MAX_EXPANDED_BYTES,
        ).is_err());
    }
}

#[test]
fn_outer_integrity_and_publisher_pins_are_independent_checks() {
    let (mut bytes, mut deployment) = archive("poster.txt", b"fixture", false);
    deployment.packages[0].publisher_key = crypto::hex(&[1; 32]);
    assert!(bundle::VerifiedBundle::read(
        &bytes,
        &deployment.packages[0],
        &deployment.scope,
        policy::MAX_EXPANDED_BYTES,
    ).is_err());
    bytes[0] ^= 1;
    assert!(bundle::VerifiedBundle::read(
        &bytes,
        &deployment.packages[0],
        &deployment.scope,
        policy::MAX_EXPANDED_BYTES,
    ).is_err());
}
