use std::io::{Cursor, Read, Write};

use protocol::{ResourcePackArchive, ResourcePackHandoff};
use uuid::Uuid;
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::{AdmissionError, validate_handoff};

const PACK_ID: Uuid = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);

fn archive(bytes: Vec<u8>) -> ResourcePackArchive {
    ResourcePackArchive::unencrypted(PACK_ID, "1.2.3".into(), String::new(), bytes)
}

fn valid_zip() -> Vec<u8> {
    let manifest = format!(
        r#"{{"format_version":2,"header":{{"name":"test","description":"test","uuid":"{PACK_ID}","version":[1,2,3]}},"modules":[{{"type":"resources","uuid":"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee","version":[1,2,3]}}]}}"#
    );
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn empty_production_handoff_is_validated_without_archives() {
    let stack = validate_handoff(ResourcePackHandoff::default()).expect("empty handoff");
    assert!(stack.packs().is_empty());
}

#[test]
fn nonempty_production_handoff_preserves_selected_metadata() {
    let handoff = ResourcePackHandoff::from_archives(vec![archive(valid_zip())]);
    let stack = validate_handoff(handoff).expect("valid handoff");
    assert_eq!(stack.packs().len(), 1);
    assert_eq!(stack.packs()[0].pack_id(), PACK_ID);
    assert_eq!(stack.packs()[0].version(), "1.2.3");
    assert_eq!(stack.packs()[0].sub_pack_name(), "");
}

#[test]
fn nonempty_production_handoff_rejects_malformed_archive_atomically() {
    let handoff = ResourcePackHandoff::from_archives(vec![archive(vec![0; 32])]);
    assert!(matches!(
        validate_handoff(handoff),
        Err(AdmissionError::InvalidZipFooter)
    ));
}

fn language_zip(path: &str, value: &[u8]) -> Vec<u8> {
    let mut zip = ZipArchive::new(Cursor::new(valid_zip())).unwrap();
    let mut manifest = String::new();
    zip.by_index(0)
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    language_zip_entries(&manifest, &[(path, value)])
}

fn language_zip_entries(manifest: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for (path, value) in entries {
        writer
            .start_file(
                *path,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(value).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn optional_preparation_is_single_use_capped_and_never_changes_admission() {
    let handoff = ResourcePackHandoff::from_archives(vec![archive(language_zip(
        "texts/en_US.lang",
        b"k=value",
    ))]);
    let (stack, value) =
        super::validate_handoff_with_file(handoff, "texts/en_US.lang", 7, |size, read| {
            assert_eq!(size, 7);
            let mut value = [0; 7];
            assert!(read(&mut value));
            assert!(!read(&mut value));
            Some(value)
        })
        .unwrap();
    assert_eq!(stack.packs().len(), 1);
    assert_eq!(value, Some(*b"k=value"));
    let handoff = ResourcePackHandoff::from_archives(vec![archive(language_zip(
        "texts/en_US.lang",
        b"k=value",
    ))]);
    let (stack, value) =
        super::validate_handoff_with_file(handoff, "texts/en_US.lang", 6, |_, _| -> Option<()> {
            panic!("oversized candidate")
        })
        .unwrap();
    assert_eq!(stack.packs().len(), 1);
    assert!(value.is_none());
    let handoff = ResourcePackHandoff::from_archives(vec![archive(language_zip(
        "texts/EN_us.lang",
        b"k=value",
    ))]);
    let (_, value) =
        super::validate_handoff_with_file(handoff, "texts/en_US.lang", 7, |_, _| -> Option<()> {
            panic!("case alias")
        })
        .unwrap();
    assert!(value.is_none());
}

#[test]
fn empty_or_multiple_archives_never_prepare_a_language_candidate() {
    let (_, value) = super::validate_handoff_with_file(
        ResourcePackHandoff::default(),
        "texts/en_US.lang",
        7,
        |_, _| -> Option<()> { panic!("empty candidate") },
    )
    .unwrap();
    assert!(value.is_none());
    let other = Uuid::from_u128(0x22222222_3333_4444_5555_666666666666);
    let mut zip = ZipArchive::new(Cursor::new(valid_zip())).unwrap();
    let mut manifest = String::new();
    zip.by_index(0)
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let manifest = manifest.replacen(&PACK_ID.to_string(), &other.to_string(), 1);
    let second = ResourcePackArchive::unencrypted(
        other,
        "1.2.3".into(),
        String::new(),
        language_zip_entries(&manifest, &[]),
    );
    let handoff = ResourcePackHandoff::from_archives(vec![
        archive(language_zip("texts/en_US.lang", b"k=value")),
        second,
    ]);
    let result =
        super::validate_handoff_with_file(handoff, "texts/en_US.lang", 7, |_, _| -> Option<()> {
            panic!("multiple candidate")
        });
    let (stack, candidate) = result.unwrap();
    assert_eq!(stack.packs().len(), 2);
    assert!(candidate.is_none());
}

#[test]
fn corrupt_optional_file_falls_back_without_rejecting_the_pack() {
    let marker = b"key=unique-value";
    let mut bytes = language_zip("texts/en_US.lang", marker);
    let offset = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .unwrap();
    bytes[offset] ^= 1;
    let handoff = ResourcePackHandoff::from_archives(vec![archive(bytes)]);
    let (stack, value) = super::validate_handoff_with_file(
        handoff,
        "texts/en_US.lang",
        marker.len(),
        |size, read| {
            let mut bytes = vec![0; size];
            read(&mut bytes).then_some(bytes)
        },
    )
    .unwrap();
    assert_eq!(stack.packs().len(), 1);
    assert!(value.is_none());
}

#[test]
fn selected_subpack_namespace_is_used_and_later_graph_refusal_drops_candidate() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Candidate(Arc<AtomicUsize>);
    impl Drop for Candidate {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let mut zip = ZipArchive::new(Cursor::new(valid_zip())).unwrap();
    let mut manifest = String::new();
    zip.by_index(0)
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let selected_manifest = format!(
        "{},\"subpacks\":[{{\"folder_name\":\"selected\",\"name\":\"selected\",\"memory_tier\":1}}]}}",
        manifest.trim_end_matches('}')
    );
    let bytes = language_zip_entries(
        &selected_manifest,
        &[
            ("texts/en_US.lang", b"root"),
            ("subpacks/selected/texts/en_US.lang", b"selected"),
        ],
    );
    let pack = ResourcePackArchive::unencrypted(PACK_ID, "1.2.3".into(), "selected".into(), bytes);
    let (_, text) = super::validate_handoff_with_file(
        ResourcePackHandoff::from_archives(vec![pack]),
        "texts/en_US.lang",
        8,
        |size, read| {
            let mut bytes = vec![0; size];
            read(&mut bytes).then_some(bytes)
        },
    )
    .unwrap();
    assert_eq!(text.as_deref(), Some(b"selected".as_slice()));
    let missing = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff";
    let manifest = format!(
        "{},\"dependencies\":[{{\"uuid\":\"{missing}\",\"version\":[1,2,3]}}]}}",
        manifest.trim_end_matches('}')
    );
    let bytes = language_zip_entries(&manifest, &[("texts/en_US.lang", b"k=value")]);
    let drops = Arc::new(AtomicUsize::new(0));
    let result = super::validate_handoff_with_file(
        ResourcePackHandoff::from_archives(vec![archive(bytes)]),
        "texts/en_US.lang",
        7,
        |_, _| Some(Candidate(Arc::clone(&drops))),
    );
    assert!(matches!(result, Err(AdmissionError::InvalidDependencies)));
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn empty_file_still_checks_crc_and_refused_preparation_stays_admitted() {
    let mut bytes = language_zip("texts/en_US.lang", b"");
    let central = bytes
        .windows(4)
        .enumerate()
        .rfind(|(_, value)| *value == b"PK\x01\x02")
        .unwrap()
        .0;
    bytes[central + 16..central + 20].copy_from_slice(&1u32.to_le_bytes());
    let (_, text) = super::validate_handoff_with_file(
        ResourcePackHandoff::from_archives(vec![archive(bytes)]),
        "texts/en_US.lang",
        0,
        |size, read| {
            assert_eq!(size, 0);
            assert!(!read(&mut []));
            None::<()>
        },
    )
    .unwrap();
    assert!(text.is_none());
    let bytes = language_zip("texts/en_US.lang", b"k=value");
    let (stack, text) = super::validate_handoff_with_file(
        ResourcePackHandoff::from_archives(vec![archive(bytes)]),
        "texts/en_US.lang",
        7,
        |_, _| None::<()>,
    )
    .unwrap();
    assert_eq!(stack.packs().len(), 1);
    assert!(text.is_none());
}

#[test]
fn short_and_excess_decoded_file_data_refuse_only_application() {
    for declared in [6u32, 8] {
        let mut bytes = language_zip("texts/en_US.lang", b"k=value");
        let central = bytes
            .windows(4)
            .enumerate()
            .rfind(|(_, value)| *value == b"PK\x01\x02")
            .unwrap()
            .0;
        bytes[central + 24..central + 28].copy_from_slice(&declared.to_le_bytes());
        let (stack, candidate) = super::validate_handoff_with_file(
            ResourcePackHandoff::from_archives(vec![archive(bytes)]),
            "texts/en_US.lang",
            8,
            |size, read| {
                let mut output = vec![0; size];
                read(&mut output).then_some(output)
            },
        )
        .unwrap();
        assert_eq!(stack.packs().len(), 1);
        assert!(candidate.is_none());
    }
}

#[test]
fn highly_compressed_oversized_language_is_refused_before_preparation() {
    let mut zip = ZipArchive::new(Cursor::new(valid_zip())).unwrap();
    let mut manifest = String::new();
    zip.by_index(0)
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    writer
        .start_file(
            "texts/en_US.lang",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )
        .unwrap();
    let cap = 1024 * 1024;
    writer.write_all(&vec![b'x'; cap + 1]).unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    assert!(bytes.len() < cap / 4);
    let (stack, candidate) = super::validate_handoff_with_file(
        ResourcePackHandoff::from_archives(vec![archive(bytes)]),
        "texts/en_US.lang",
        cap,
        |_, _| -> Option<()> { panic!("oversized file preparation") },
    )
    .unwrap();
    assert_eq!(stack.packs().len(), 1);
    assert!(candidate.is_none());
}
