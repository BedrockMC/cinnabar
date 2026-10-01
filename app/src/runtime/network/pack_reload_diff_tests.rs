use super::*;
use std::{
    io::{Cursor, Write},
    sync::Arc,
};
use zip::{ZipWriter, write::SimpleFileOptions};

/// Builds synthetic UI layers without external pack assets.
fn stack(files: &[(&str, &[u8])]) -> Arc<ValidatedPackStack> {
    super::super::pack_reload_tests::stack(files)
}

/// Resolves the same per-pack UI order used by runtime catalog publication.
fn control_text(stack: &ValidatedPackStack) -> String {
    let mut catalog = json_ui::Catalog::default();
    for pack in stack.packs() {
        let files = pack
            .files_under("ui/")
            .iter()
            .map(|path| ((*path).to_owned(), pack.read_file(path).unwrap().unwrap()))
            .collect::<Vec<_>>();
        catalog.apply_pack(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_ref())),
        );
    }
    catalog.lookup("test", "control").unwrap().props["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn layer_boundaries_invalidate_ui_when_per_pack_ui_defs_change_effective_order() {
    let defs = br#"{"ui_defs":["ui/b.json","ui/a.json"]}"#;
    let a = br#"{"namespace":"test","control":{"type":"label","text":"a"}}"#;
    let b = br#"{"namespace":"test","control":{"type":"label","text":"b"}}"#;
    let together = stack(&[
        ("ui/_ui_defs.json", defs),
        ("ui/a.json", a),
        ("ui/b.json", b),
    ]);
    let lower = stack(&[("ui/_ui_defs.json", defs), ("ui/a.json", a)]);
    let higher = stack(&[("ui/b.json", b)]);
    let split = ValidatedPackStack::compose(&lower, &higher).unwrap();
    assert_eq!(control_text(&together), "a");
    assert_eq!(control_text(&split), "b");
    let previous = PackApplication {
        admission: PackAdmission::Validated(together),
        ..Default::default()
    };
    assert!(Changes::between(&split, Some(&previous)).ui);
}

#[test]
fn empty_layers_leave_subscriber_fingerprints_unchanged() {
    let source = stack(&[("ui/test.json", b"{}")]);
    let empty = stack(&[]);
    let together = ValidatedPackStack::compose(&source, &empty).unwrap();
    let previous = PackApplication {
        admission: PackAdmission::Validated(source),
        ..Default::default()
    };
    assert!(!Changes::between(&together, Some(&previous)).ui);
}

#[test]
fn selected_subpack_logical_content_changes_the_fingerprint() {
    let id = "00000000-0000-0000-0000-000000000033";
    let manifest = format!(
        r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}],"subpacks":[{{"folder_name":"high","name":"High","memory_tier":2}}]}}"#
    );
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in [
        ("manifest.json", manifest.as_bytes()),
        ("ui/test.json", b"root".as_slice()),
        ("subpacks/high/ui/test.json", b"high".as_slice()),
    ] {
        writer
            .start_file(path, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    let admit = |subpack: &str| {
        resource_pack::validate_handoff(protocol::ResourcePackHandoff::from_archives(vec![
            protocol::ResourcePackArchive::unencrypted(
                id.parse().unwrap(),
                "1.0.0".into(),
                subpack.into(),
                bytes.clone(),
            ),
        ]))
    };
    let root = admit("");
    let high = admit("high");
    let previous = PackApplication {
        admission: PackAdmission::Validated(root),
        ..Default::default()
    };
    assert!(Changes::between(&high, Some(&previous)).ui);
}
