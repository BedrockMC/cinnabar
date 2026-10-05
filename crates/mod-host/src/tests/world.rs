use super::*;

fn source(init: &str, frame: &str) -> String {
    let package = include_str!("../../../mod-api/wit/extension.wit")
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("package ")
        .trim_end_matches(';');
    let (name, version) = package.split_once('@').unwrap();
    include_str!("world.wat")
        .replace("$GAMEPLAY", &format!("{name}/gameplay@{version}"))
        .replace("$EVENTS", &format!("{name}/events@{version}"))
        .replace("$INIT", init)
        .replace("$FRAME", frame)
}

fn load(init: &str, frame: &str) -> (tempfile::TempDir, ModHost) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("world.wat");
    std::fs::write(&path, source(init, frame)).unwrap();
    let grants = ModGrants {
        camera: true,
        commands: vec!["ability".into()],
        ..Default::default()
    };
    let host = ModHost::load_with_grants(&path, grants).unwrap();
    (directory, host)
}

fn snapshot() -> GameplaySnapshot {
    GameplaySnapshot {
        session: 1,
        dimension: 0,
        eye: GameplayVector3 {
            x: 0.0,
            y: 64.0,
            z: 0.0,
        },
        yaw: 0.0,
        pitch: 0.0,
        attack_held: false,
        frame_seconds: 0.016,
        players: Vec::new(),
    }
}

fn checked(call: &str) -> String {
    format!("{call} i32.const 256 i32.load8_u if unreachable end")
}

fn set_rig() -> String {
    checked(
        "i32.const 1 f32.const 0.75 f32.const 0.5 f32.const 3 f32.const 0.1 f32.const 4 \
        i32.const 256 call $rig",
    )
}

fn stage_all() -> String {
    format!(
        "{} {} {}",
        set_rig(),
        checked("i32.const 528 i32.const 14 i32.const 256 call $command"),
        checked("i32.const 512 i32.const 12 i32.const 600 i32.const 0 i32.const 256 call $emit")
    )
}

fn expected_rig() -> GameplayCameraRig {
    GameplayCameraRig {
        offset: GameplayVector3 {
            x: 0.75,
            y: 0.5,
            z: 3.0,
        },
        roll: 0.1,
        fov_delta: 4.0,
    }
}

#[test]
fn rig_commands_and_cues_commit_across_the_component_boundary() {
    let (_dir, mut host) = load("", &stage_all());
    host.frame_with_gameplay(false, Some(snapshot())).unwrap();
    assert_eq!(host.camera_rig(), Some(expected_rig()));
    assert_eq!(host.take_commands(), ["/ability flash"]);
    assert_eq!(
        host.take_cues(),
        [ModCue {
            name: "camera.dodge".into(),
            values: Vec::new()
        }]
    );
    assert!(host.take_commands().is_empty());
    assert!(host.take_cues().is_empty());
}

#[test]
fn trap_revokes_the_retained_rig_and_discards_staged_requests() {
    let frame = format!(
        "global.get $frames i32.const 1 i32.eq if {} else {} unreachable end",
        set_rig(),
        stage_all()
    );
    let (_dir, mut host) = load("", &frame);
    host.frame_with_gameplay(false, Some(snapshot())).unwrap();
    assert_eq!(host.camera_rig(), Some(expected_rig()));
    assert!(host.frame_with_gameplay(false, Some(snapshot())).is_err());
    assert_eq!(host.camera_rig(), None);
    assert!(host.take_commands().is_empty());
    assert!(host.take_cues().is_empty());
}

#[test]
fn reload_starts_without_the_previous_rig() {
    let (directory, mut host) = load("", &set_rig());
    host.frame_with_gameplay(false, Some(snapshot())).unwrap();
    assert_eq!(host.camera_rig(), Some(expected_rig()));
    std::fs::write(directory.path().join("world.wat"), source("", "")).unwrap();
    assert!(host.reload_if_changed().unwrap());
    assert_eq!(host.camera_rig(), None);
}

#[test]
fn frames_without_gameplay_cannot_set_a_rig_or_request_commands() {
    let (_dir, mut host) = load("", &stage_all());
    assert!(host.frame(false).is_err(), "the guest traps on the denial");
    assert_eq!(host.camera_rig(), None);
    assert!(host.take_commands().is_empty());
}

/// The packaged showcase mod drives every new import through the real ABI.
#[test]
fn packaged_showcase_mod_requests_abilities_and_locks_on() {
    let Some(path) = std::env::var_os("CINNABAR_SHOWCASE_COMPONENT") else {
        eprintln!(
            "skipping packaged_showcase_mod_requests_abilities_and_locks_on: fixture unavailable; \
            requires CINNABAR_SHOWCASE_COMPONENT (build showcase-camera-mod and mod-host pack)"
        );
        return;
    };
    let grants = ModGrants {
        players: true,
        camera: true,
        controls: true,
        entities: true,
        commands: vec!["ability".into()],
        ..Default::default()
    };
    let mut host = ModHost::load_with_grants(std::path::Path::new(&path), grants).unwrap();
    assert!(host.reserved_keys().contains(&"Digit1".to_owned()));
    let boss = GameplayMob {
        runtime_id: 9,
        unique_id: 9,
        type_id: "cinnabar:hollow_warden".into(),
        position: GameplayVector3 {
            x: 10.0,
            y: 64.0,
            z: 0.0,
        },
        health: Some(300.0),
        max_health: Some(400.0),
    };
    let mut controls = crate::empty_controls();
    controls.seconds = 0.016;
    controls.gameplay = true;
    controls.keys_pressed = vec!["KeyR".into(), "Digit3".into()];
    host.frame_with_world(false, Some(snapshot()), vec![boss], controls)
        .unwrap();
    assert!(host.is_active());
    assert_eq!(host.take_commands(), ["/ability flash"]);
    let cues: Vec<_> = host.take_cues().into_iter().map(|cue| cue.name).collect();
    assert_eq!(cues, ["lockon.on", "ability.flash"]);
    let rig = host.camera_rig().unwrap();
    assert!(rig.offset.x > 0.0 && rig.offset.z > 0.0);
    assert!(
        host.take_camera_delta()
            .is_some_and(|delta| delta.yaw < 0.0)
    );
    assert_eq!(host.label(), Some("Lock-on: Hollow Warden"));
}
