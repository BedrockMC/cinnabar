use resource_pack::{AdmissionError, PackAdmission};

use super::ResourcePackAdmissionState;

/// Serializes tests that go through the process-wide block overlay cache.
static OVERLAY_CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn overlay_cache() -> std::sync::MutexGuard<'static, ()> {
    OVERLAY_CACHE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[test]
fn absent_or_rejected_application_preserves_optional_admission() {
    let _cache = overlay_cache();
    let application = super::prepare_pack_application(
        protocol::ResourcePackHandoff::default(),
        &protocol::CustomBlocks::default(),
        &[],
        &[],
        false,
    );
    assert!(matches!(application.admission, PackAdmission::None));
    assert!(application.server_lang.is_none());
    let pack = protocol::ResourcePackArchive::unencrypted(
        "11111111-2222-3333-4444-555555555555".parse().unwrap(),
        "1.2.3".into(),
        String::new(),
        vec![0; 32],
    );
    let application = super::prepare_pack_application(
        protocol::ResourcePackHandoff::from_archives(vec![pack]),
        &protocol::CustomBlocks::default(),
        &[],
        &[],
        false,
    );
    let overlay = application.server_lang;
    let PackAdmission::Validated(stack) = application.admission else {
        panic!("a dropped pack still yields an admitted stack");
    };
    assert!(stack.packs().is_empty());
    assert_eq!(
        stack.rejections()[0].reason,
        AdmissionError::InvalidZipFooter
    );
    assert!(overlay.is_none());
}

// A required pack that fails validation refuses the join in vanilla's words;
// an optional one is dropped and the join goes on.
#[test]
fn a_rejected_required_pack_refuses_the_join() {
    let _cache = overlay_cache();
    for required in [false, true] {
        let broken = protocol::ResourcePackArchive::unencrypted(
            "11111111-2222-3333-4444-555555555555".parse().unwrap(),
            "1.2.3".into(),
            String::new(),
            vec![0; 32],
        );
        let handoff =
            protocol::ResourcePackHandoff::from_archives(vec![broken]).with_required(required);
        assert_eq!(handoff.required(), required);
        let application = super::prepare_pack_application(
            handoff,
            &protocol::CustomBlocks::default(),
            &[],
            &[],
            false,
        );
        let outcome = super::required_packs_applied(required, &application.admission);
        assert_eq!(outcome.is_err(), required);
        if let Err(error) = outcome {
            let failure =
                crate::runtime::network::session_failure_display(&error.to_string(), None);
            assert_eq!(
                crate::menu::disconnect::describe(&failure).body,
                crate::menu::disconnect::DisconnectBody::Key("disconnectionScreen.resourcePack")
            );
        }
    }
}

fn lang_pack(id: u128, lang: &[u8]) -> protocol::ResourcePackArchive {
    archive(id, &[("texts/en_US.lang", lang)])
}

fn archive(id: u128, files: &[(&str, &[u8])]) -> protocol::ResourcePackArchive {
    use std::io::Write;
    let id = format!("00000000-0000-0000-0000-{id:012x}");
    let manifest = format!(
        r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, bytes) in
        std::iter::once(("manifest.json", manifest.as_bytes())).chain(files.iter().copied())
    {
        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let archive = writer.finish().unwrap().into_inner();
    protocol::ResourcePackArchive::unencrypted(
        id.parse().unwrap(),
        "1.0.0".into(),
        String::new(),
        archive,
    )
}

fn files_pack(files: &[(&str, &[u8])]) -> resource_pack::LayeredPackView {
    use std::io::Write;
    let id = "00000000-0000-0000-0000-00000000000a";
    let manifest = format!(
        r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
    );
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, bytes) in
        std::iter::once(("manifest.json", manifest.as_bytes())).chain(files.iter().copied())
    {
        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    let archive = protocol::ResourcePackArchive::unencrypted(
        id.parse().unwrap(),
        "1.0.0".into(),
        String::new(),
        writer.finish().unwrap().into_inner(),
    );
    resource_pack::LayeredPackView::new(resource_pack::validate_handoff(
        protocol::ResourcePackHandoff::from_archives(vec![archive]),
    ))
}

// A cancelled session's late preparation must not replace the live session's sounds.
#[test]
fn preparing_packs_leaves_sound_publication_to_bootstrap() {
    let _cache = overlay_cache();
    let _mailbox = crate::audio::SERVER_SOUNDS_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = crate::audio::server_sounds_generation();
    let sounds = br#"{"sound_definitions":{"custom.beep":{"sounds":["sounds/beep"]}}}"#;
    let application = super::prepare_pack_application(
        protocol::ResourcePackHandoff::from_archives(vec![archive(
            3,
            &[("sounds/sound_definitions.json", sounds)],
        )]),
        &protocol::CustomBlocks::default(),
        &[],
        &[],
        false,
    );
    assert_eq!(crate::audio::server_sounds_generation(), before);
    assert!(application.server_sounds.is_some(), "carried to Bootstrap");
}

// A texture set resolves to its sibling color image or a solid color.
#[test]
fn texture_sets_supply_color_layers() {
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let png = png.into_inner();
    let view = files_pack(&[
        (
            "textures/blocks/a.texture_set.json",
            br#"{"format_version":"1.16.100","minecraft:texture_set":{"color":"a_color"}}"#,
        ),
        ("textures/blocks/a_color.png", &png),
        (
            "textures/blocks/b.texture_set.json",
            br#"{"minecraft:texture_set":{"color":[10,20,30]}}"#,
        ),
    ]);
    let image = super::decode_pack_texture(&view, "textures/blocks/a").expect("sibling");
    assert_eq!(
        (image.width, image.height, image.rgba8[..4].to_vec()),
        (2, 2, vec![1, 2, 3, 255])
    );
    let solid = super::decode_pack_texture(&view, "textures/blocks/b").expect("solid");
    assert_eq!(solid.rgba8.to_vec(), vec![10, 20, 30, 255]);
}

// Higher packs override shared keys; keys only a lower pack defines survive.
#[test]
fn language_files_merge_across_the_stack_by_precedence() {
    let _cache = overlay_cache();
    let handoff = protocol::ResourcePackHandoff::from_archives(vec![
        lang_pack(2, b"\xef\xbb\xbfshared=bottom\nbottom.only=B"),
        lang_pack(1, b"shared=top\ntop.only=T"),
    ]);
    let application = super::prepare_pack_application(
        handoff,
        &protocol::CustomBlocks::default(),
        &[],
        &[],
        false,
    );
    let overlay = application.server_lang.expect("merged overlay");
    assert_eq!(overlay.lookup("shared"), Some("top"));
    assert_eq!(overlay.lookup("top.only"), Some("T"));
    assert_eq!(overlay.lookup("bottom.only"), Some("B"));
}

// The same stack and blocks reuse the compiled overlay instead of recompiling.
#[test]
fn overlay_cache_reuses_the_previous_session_compile() {
    let _cache = overlay_cache();
    let blocks = protocol::CustomBlocks {
        blocks: vec![protocol::CustomBlock {
            name: "cache:test".into(),
            state_count: 1,
            collides: true,
            collision_box: None,
            selection: Default::default(),
            visual: Default::default(),
        }]
        .into(),
        skipped: 0,
    };
    let stack =
        resource_pack::validate_handoff(protocol::ResourcePackHandoff::from_archives(vec![
            lang_pack(7, b"a=b"),
        ]));
    let mut compiles = 0;
    let fingerprint = super::stack_fingerprint(&stack);
    let view = resource_pack::LayeredPackView::tracked(stack);
    for _ in 0..2 {
        super::cached_block_overlay(&fingerprint, &view, &blocks, false, || {
            compiles += 1;
            None
        });
    }
    assert_eq!(compiles, 1);
    super::cached_block_overlay(&fingerprint, &view, &blocks, true, || {
        compiles += 1;
        None
    });
    assert_eq!(compiles, 2);
}

#[test]
fn newer_generation_replaces_atomically_and_stale_results_are_ignored() {
    let mut state = ResourcePackAdmissionState::default();
    assert!(state.begin_generation(2));
    assert!(matches!(state.admission(), PackAdmission::None));
    let stack = resource_pack::validate_handoff(protocol::ResourcePackHandoff::default());
    assert!(state.replace_for_generation(2, PackAdmission::Validated(stack)));
    assert!(!state.replace_for_generation(1, PackAdmission::None));
    assert_eq!(state.generation(), 2);
    assert!(matches!(state.admission(), PackAdmission::Validated(_)));
    assert!(state.begin_generation(3));
    assert!(matches!(state.admission(), PackAdmission::None));
    assert!(!state.begin_generation(2));
    state.clear_current();
    assert_eq!(state.generation(), 3);
    assert!(matches!(state.admission(), PackAdmission::None));
}
