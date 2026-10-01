use super::super::{ServerUiPack, pack_harness, snapshot, tests::mini_engine_presentation};
use super::*;
use render::UiRenderInput;
use ui::DpiScale;

/// A real engine over a small HUD fixture, independent of local carrier files.
fn presentation() -> UiPresentationRuntime {
    let mut presentation = mini_engine_presentation();
    presentation.set_server_ui_pack(&ServerUiPack {
        ui_layers: vec![vec![(
            "ui/hud_screen.json".to_owned(),
            br#"{
            "namespace": "hud",
            "hud_screen": { "type": "label", "size": [100, 12],
                "anchor_from": "bottom_middle", "anchor_to": "bottom_middle",
                "text": "Base HUD" }
        }"#
            .to_vec(),
        )]],
        ..Default::default()
    });
    presentation
}

/// Builds the same offline frame so only the extension state can change its pixels.
fn frame(presentation: &mut UiPresentationRuntime) -> UiRenderInput {
    presentation
        .build(
            &UiRuntime::new(1),
            0,
            [1280, 720],
            DpiScale::new(1.0).unwrap(),
        )
        .unwrap()
}

#[test]
fn no_mod_output_is_identical_to_the_vanilla_frame() {
    let mut presentation = presentation();
    let before = frame(&mut presentation);
    presentation.set_mod_label(None).unwrap();
    let after = frame(&mut presentation);
    assert_eq!(before, after);
    assert!(presentation.form_presentation.mod_hud.is_none());
}

#[test]
fn extension_mount_update_and_revoke_use_json_ui() {
    let mut presentation = presentation();
    let before = snapshot::rasterize(&frame(&mut presentation));
    presentation
        .set_mod_label(Some("Cinnabar extension: Hello"))
        .unwrap();
    let first = frame(&mut presentation);
    assert_ne!(before, snapshot::rasterize(&first));
    assert_eq!(first, frame(&mut presentation));
    assert_eq!(
        presentation
            .form_presentation
            .mod_hud
            .as_ref()
            .unwrap()
            .screen
            .passes,
        1
    );
    presentation
        .set_mod_label(Some("Cinnabar extension: F8 pressed"))
        .unwrap();
    let updated = frame(&mut presentation);
    assert_ne!(first.vertices, updated.vertices);
    presentation.set_mod_label(None).unwrap();
    assert_eq!(before, snapshot::rasterize(&frame(&mut presentation)));
}

#[test]
fn extension_is_hidden_while_chat_or_inventory_owns_input() {
    for inventory in [false, true] {
        let mut presentation = presentation();
        let mut runtime = UiRuntime::new(1);
        runtime.inventory_open = inventory;
        runtime.chat_focused = !inventory;
        let build = |presentation: &mut UiPresentationRuntime| {
            presentation
                .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
                .unwrap()
        };
        let before = snapshot::rasterize(&build(&mut presentation));
        presentation
            .set_mod_label(Some("Hidden extension"))
            .unwrap();
        assert_eq!(before, snapshot::rasterize(&build(&mut presentation)));
        assert_eq!(
            presentation
                .form_presentation
                .mod_hud
                .as_ref()
                .unwrap()
                .screen
                .passes,
            0
        );
    }
}

#[test]
fn extension_cannot_restore_a_server_hidden_hud() {
    let mut presentation = presentation();
    presentation.set_server_ui_pack(&ServerUiPack {
        ui_layers: vec![vec![(
            "ui/hud_screen.json".to_owned(),
            br#"{
            "namespace": "hud", "hud_screen": { "type": "panel", "visible": false }
        }"#
            .to_vec(),
        )]],
        ..Default::default()
    });
    let before = frame(&mut presentation);
    presentation
        .set_mod_label(Some("Hidden extension"))
        .unwrap();
    assert_eq!(before, frame(&mut presentation));
    assert_eq!(
        presentation
            .form_presentation
            .mod_hud
            .as_ref()
            .unwrap()
            .screen
            .passes,
        0
    );
}

#[test]
fn mod_spike_snapshot_with_real_carrier() {
    let Some(mut presentation) = pack_harness::engine_presentation() else {
        assert!(
            std::env::var_os("CINNABAR_FORM_SNAPSHOT_DIR").is_none(),
            "snapshot requested without UI carrier"
        );
        return;
    };
    let before = frame(&mut presentation);
    snapshot::write(&before, "mod-spike-before");
    let mut guest = std::env::var_os("CINNABAR_MOD_SNAPSHOT_COMPONENT")
        .map(|path| mod_host::ModHost::load(std::path::Path::new(&path)).unwrap());
    let text = guest
        .as_ref()
        .map(|host| host.label().expect("sample must publish a label"))
        .unwrap_or("Cinnabar extension: Hello (Press demo key)");
    presentation.set_mod_label(Some(text)).unwrap();
    let after = frame(&mut presentation);
    snapshot::write(&after, "mod-spike-after");
    assert_ne!(snapshot::rasterize(&before), snapshot::rasterize(&after));
    if let Some(host) = guest.as_mut() {
        host.frame(true).unwrap();
        presentation.set_mod_label(host.label()).unwrap();
        let pressed = frame(&mut presentation);
        snapshot::write(&pressed, "mod-spike-keybind");
        assert_ne!(snapshot::rasterize(&after), snapshot::rasterize(&pressed));
    }
    presentation.set_mod_label(None).unwrap();
    assert_eq!(
        snapshot::rasterize(&before),
        snapshot::rasterize(&frame(&mut presentation))
    );
}
