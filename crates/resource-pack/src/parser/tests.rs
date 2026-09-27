use std::io::Write;

use zip::{ZipWriter, write::SimpleFileOptions};

use super::*;

const PACK_ID: Uuid = Uuid::from_u128(0x11111111_2222_3333_4444_555555555555);
const MODULE_ID: Uuid = Uuid::from_u128(0xaaaaaaaa_bbbb_cccc_dddd_eeeeeeeeeeee);

fn manifest(extra: &str) -> String {
    format!(
        r#"{{
            // numeric v2 manifests are admitted
            "format_version": 2,
            "header": {{
                "name": "test // literal",
                "description": "bounded",
                "uuid": "{PACK_ID}",
                "version": [1, 2, 3]
            }},
            "modules": [{{
                "type": "resources",
                "uuid": "{MODULE_ID}",
                "version": [1, 2, 3]
            }}]
            {extra}
        }}"#
    )
}

fn zip_files(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in files {
        writer
            .start_file(*path, SimpleFileOptions::default())
            .expect("start fixture file");
        writer.write_all(bytes).expect("write fixture file");
    }
    writer.finish().expect("finish fixture ZIP").into_inner()
}

fn deflated_zip_file(path: &str, bytes: &[u8]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            path,
            SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
        )
        .expect("start deflated fixture file");
    writer
        .write_all(bytes)
        .expect("write deflated fixture file");
    writer.finish().expect("finish fixture ZIP").into_inner()
}

fn validate_fixture(bytes: Vec<u8>, selected: &str) -> Result<ValidatedPack, AdmissionError> {
    validate_archive_parts(PACK_ID, "1.2.3", selected, bytes, &mut disabled_file())
        .map(|(pack, _)| pack)
}
#[test]
fn admits_jsonc_manifest_and_exposes_only_selected_logical_namespace() {
    let manifest = manifest(
        r#", "subpacks": [
            {"folder_name":"low", "name":"Low", "memory_tier":1},
            {"folder_name":"high", "name":"High", "memory_tier":2}
        ] /* a block comment */"#,
    );
    let archive = zip_files(&[
        ("manifest.json", manifest.as_bytes()),
        ("textures/root.txt", b"root"),
        ("subpacks/low/textures/root.txt", b"low"),
        ("subpacks/high/textures/root.txt", b"high"),
        ("subpacks/high/textures/only.txt", b"only"),
    ]);
    let pack = validate_fixture(archive, "high").expect("valid selected subpack");

    assert_eq!(
        pack.read_file("textures/root.txt")
            .unwrap()
            .unwrap()
            .as_ref(),
        b"high"
    );
    assert_eq!(
        pack.read_file("textures/only.txt")
            .unwrap()
            .unwrap()
            .as_ref(),
        b"only"
    );
    assert!(
        pack.read_file("subpacks/low/textures/root.txt")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        pack.files_under("textures/").as_ref(),
        ["textures/only.txt", "textures/root.txt"]
    );
}
#[test]
fn root_selection_excludes_all_physical_subpacks() {
    let manifest =
        manifest(r#", "subpacks": [{"folder_name":"high", "name":"High", "memory_tier":2}]"#);
    let archive = zip_files(&[
        ("manifest.json", manifest.as_bytes()),
        ("base.txt", b"root"),
        ("subpacks/high/base.txt", b"high"),
    ]);
    let pack = validate_fixture(archive, "").expect("root selection");
    assert_eq!(
        pack.read_file("base.txt").unwrap().unwrap().as_ref(),
        b"root"
    );
    assert_eq!(pack.files_under("").as_ref(), ["base.txt", "manifest.json"]);
}
#[test]
fn rejects_unsafe_duplicate_and_nonfile_entries() {
    let manifest = manifest("");
    let traversal = zip_files(&[("manifest.json", manifest.as_bytes()), ("../x", b"x")]);
    assert_eq!(
        validate_fixture(traversal, "").unwrap_err(),
        AdmissionError::UnsafePath
    );

    let collision = zip_files(&[
        ("manifest.json", manifest.as_bytes()),
        ("Textures/x", b"a"),
        ("textures/X", b"b"),
    ]);
    assert_eq!(
        validate_fixture(collision, "").unwrap_err(),
        AdmissionError::DuplicatePath
    );

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_directory("directory/", SimpleFileOptions::default())
        .unwrap();
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    let directory = writer.finish().unwrap().into_inner();
    assert_eq!(
        validate_fixture(directory, "").unwrap_err(),
        AdmissionError::NonFileEntry
    );

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink("link", "target", SimpleFileOptions::default())
        .unwrap();
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    let symlink = writer.finish().unwrap().into_inner();
    assert_eq!(
        validate_fixture(symlink, "").unwrap_err(),
        AdmissionError::NonFileEntry
    );
}
#[test]
fn rejects_zip64_sentinel_before_zip_parser_allocation() {
    let manifest = manifest("");
    let mut archive = zip_files(&[("manifest.json", manifest.as_bytes())]);
    let eocd = archive
        .windows(4)
        .rposition(|window| window == b"PK\x05\x06")
        .unwrap();
    archive[eocd + 10..eocd + 12].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        preflight_eocd(&archive),
        Err(AdmissionError::UnsupportedZip64)
    );
}
#[test]
fn rejects_zip_encryption_and_unsupported_compression() {
    let manifest = manifest("");
    let original = zip_files(&[("manifest.json", manifest.as_bytes())]);

    let mut encrypted = original.clone();
    let local = encrypted
        .windows(4)
        .position(|window| window == b"PK\x03\x04")
        .unwrap();
    let central = encrypted
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    encrypted[local + 6..local + 8].copy_from_slice(&1u16.to_le_bytes());
    encrypted[central + 8..central + 10].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        validate_fixture(encrypted, "").unwrap_err(),
        AdmissionError::UnsupportedZipEncryption
    );

    let mut unsupported = original;
    unsupported[local + 8..local + 10].copy_from_slice(&12u16.to_le_bytes());
    unsupported[central + 10..central + 12].copy_from_slice(&12u16.to_le_bytes());
    assert_eq!(
        validate_fixture(unsupported, "").unwrap_err(),
        AdmissionError::UnsupportedCompression
    );
}
#[test]
fn rejects_malformed_jsonc_and_multiple_json_values() {
    for body in [
        br#"{"format_version":2 /* unterminated"#.as_slice(),
        br#"{} {}"#.as_slice(),
    ] {
        let archive = zip_files(&[("manifest.json", body)]);
        assert_eq!(
            validate_fixture(archive, "").unwrap_err(),
            AdmissionError::MalformedManifest
        );
    }

    let invalid_utf8_comment = b"// \xff\n{}";
    let archive = zip_files(&[("manifest.json", invalid_utf8_comment)]);
    assert_eq!(
        validate_fixture(archive, "").unwrap_err(),
        AdmissionError::MalformedManifest
    );
}

#[test]
fn manifest_read_is_bounded_by_its_forged_declared_size() {
    let body = manifest("");
    let mut archive = deflated_zip_file("manifest.json", body.as_bytes());
    let local = archive
        .windows(4)
        .position(|window| window == b"PK\x03\x04")
        .unwrap();
    let central = archive
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    archive[local + 22..local + 26].copy_from_slice(&1u32.to_le_bytes());
    archive[central + 24..central + 28].copy_from_slice(&1u32.to_le_bytes());

    assert_eq!(
        validate_fixture(archive, "").unwrap_err(),
        AdmissionError::InvalidFileData
    );
}

#[test]
fn rejects_cycles_in_the_exact_selected_dependency_graph() {
    let other = Uuid::from_u128(0x99999999_8888_7777_6666_555555555555);
    let first_manifest = manifest(&format!(
        r#", "dependencies": [{{"uuid":"{other}", "version":[1,2,3]}}]"#
    ));
    let second_manifest = manifest(&format!(
        r#", "dependencies": [{{"uuid":"{PACK_ID}", "version":[1,2,3]}}]"#
    ))
    .replacen(&PACK_ID.to_string(), &other.to_string(), 1);
    let first = validate_archive_parts(
        PACK_ID,
        "1.2.3",
        "",
        zip_files(&[("manifest.json", first_manifest.as_bytes())]),
        &mut disabled_file(),
    )
    .unwrap()
    .0;
    let second = validate_archive_parts(
        other,
        "1.2.3",
        "",
        zip_files(&[("manifest.json", second_manifest.as_bytes())]),
        &mut disabled_file(),
    )
    .unwrap()
    .0;
    assert_eq!(
        validate_dependency_graph(&[first, second]),
        Err(AdmissionError::DependencyCycle)
    );
}

#[test]
fn admits_public_reference_scale_entry_count() {
    const REFERENCE_ENTRY_COUNT: usize = 17_805;
    let manifest = manifest("");
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for index in 1..REFERENCE_ENTRY_COUNT {
        writer
            .start_file(
                format!("textures/generated/{index:05}.txt"),
                SimpleFileOptions::default(),
            )
            .unwrap();
    }
    let archive = writer.finish().unwrap().into_inner();
    let pack = validate_fixture(archive, "").expect("reference-scale central directory");
    assert_eq!(pack.entry_count(), REFERENCE_ENTRY_COUNT);
}

#[test]
fn maximum_entry_subpack_overlay_uses_bounded_key_index() {
    let manifest =
        manifest(r#", "subpacks": [{"folder_name":"high", "name":"High", "memory_tier":2}]"#);
    let common = format!("assets/{}/", "a".repeat(420));
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("manifest.json", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest.as_bytes()).unwrap();
    for index in 0..16_383 {
        writer
            .start_file(
                format!("{common}{index:05}.txt"),
                SimpleFileOptions::default(),
            )
            .unwrap();
    }
    for index in 0..16_384 {
        writer
            .start_file(
                format!("subpacks/high/{common}{index:05}.txt"),
                SimpleFileOptions::default(),
            )
            .unwrap();
    }
    let archive = writer.finish().unwrap().into_inner();
    let pack = validate_fixture(archive, "high").expect("maximum-entry selected subpack");
    assert_eq!(pack.entry_count(), MAX_ENTRIES_PER_PACK);
    assert_eq!(pack.files_under("assets/").len(), 16_384);
}

#[test]
fn errors_never_include_manifest_or_path_data() {
    let secret = "attacker-secret-marker";
    let archive = zip_files(&[("manifest.json", secret.as_bytes())]);
    let error = validate_fixture(archive, "").unwrap_err();
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
}
