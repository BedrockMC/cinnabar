use super::*;
use server_experience::manifest::{Permission, Scope};
use std::collections::BTreeSet;

/// Creates the host import state with worst-case serialized identity and epoch fields.
fn state() -> State {
    let mut state = State {
        limits: StoreLimitsBuilder::new().build(),
        owner: Principal {
            session: "\\\"".repeat(MAX_IDENTIFIER_BYTES),
            bundle: "b".repeat(MAX_IDENTIFIER_BYTES),
            generation: u64::MAX,
        },
        epoch: u64::MAX,
        capabilities: Capabilities {
            scope: Scope {
                permissions: BTreeSet::from([Permission::Ui]),
                origins: BTreeSet::new(),
                memory_bytes: 0,
                gpu_bytes: 0,
            },
            assets: BTreeSet::new(),
            templates: BTreeSet::new(),
            channels: Vec::new(),
            actions: BTreeSet::new(),
        },
        action: None,
        commands: Vec::new(),
        bytes: 0,
        calls: 0,
    };
    state.begin_output().unwrap();
    state
}

#[test]
fn staged_output_at_the_transaction_boundary_round_trips_through_ipc() {
    let mut state = state();
    let command = Command::Widget {
        id: "w".repeat(MAX_IDENTIFIER_BYTES),
        text: "x".repeat(MAX_WIDGET_TEXT_BYTES),
    };
    let widget = |text: usize| Command::Widget {
        id: "w".repeat(MAX_IDENTIFIER_BYTES),
        text: "x".repeat(text),
    };
    let empty = widget(0);
    let overhead = serde_json::to_vec(&empty).unwrap().len() + 1;
    // Each filler leaves room for at least one more command, so the last fits exactly.
    while MAX_HOST_OUTPUT - state.bytes > overhead + MAX_WIDGET_TEXT_BYTES {
        let room = MAX_HOST_OUTPUT - state.bytes - 2 * overhead;
        state
            .stage(widget(room.min(MAX_WIDGET_TEXT_BYTES)))
            .unwrap()
            .unwrap();
    }
    let text = "x".repeat(MAX_HOST_OUTPUT - state.bytes - overhead);
    assert!(text.len() <= MAX_WIDGET_TEXT_BYTES);
    state
        .stage(Command::Widget {
            id: "w".repeat(MAX_IDENTIFIER_BYTES),
            text,
        })
        .unwrap()
        .unwrap();
    assert_eq!(state.bytes, MAX_HOST_OUTPUT);
    let count = state.commands.len();
    assert!(state.stage(empty).unwrap().is_err());
    assert_eq!(state.commands.len(), count);
    let transaction = Transaction {
        owner: state.owner.clone(),
        epoch: state.epoch,
        commands: state.commands.clone(),
    };
    assert_eq!(
        serde_json::to_vec(&transaction).unwrap().len(),
        MAX_HOST_OUTPUT
    );
    let mut frame = Vec::new();
    crate::helper::write_frame(&mut frame, &transaction, MAX_HOST_OUTPUT).unwrap();
    let decoded: Transaction =
        crate::helper::read_frame(&mut frame.as_slice(), MAX_HOST_OUTPUT).unwrap();
    assert_eq!(decoded.owner, transaction.owner);
    assert_eq!(decoded.commands.len(), count);
    state.epoch = 0;
    state.begin_output().unwrap();
    state.stage(command).unwrap().unwrap();
    let transaction = Transaction {
        owner: state.owner,
        epoch: state.epoch,
        commands: state.commands,
    };
    assert_eq!(serde_json::to_vec(&transaction).unwrap().len(), state.bytes);
}

/// The 1.1 test guest, its imports named from the WIT package the host binds.
fn guest_1_1(template: &str) -> String {
    let source = include_str!("../../../mod-api/wit/deps/server-experience/capabilities.wit");
    let package = source
        .lines()
        .next()
        .unwrap()
        .trim_start_matches("package ")
        .trim_end_matches(';');
    let (name, version) = package.split_once('@').unwrap();
    include_str!("guest_1_1.wat")
        .replace("$UI", &format!("{name}/ui@{version}"))
        .replace("$INPUT", &format!("{name}/input@{version}"))
        .replace("$TEMPLATE_LENGTH", &template.len().to_string())
        .replace("$TEMPLATE", template)
}

fn owner() -> Principal {
    Principal {
        session: "s".into(),
        bundle: "demo".into(),
        generation: 1,
    }
}

fn screen_capabilities(permissions: &[Permission]) -> Capabilities {
    Capabilities {
        scope: Scope {
            permissions: permissions.iter().copied().collect(),
            origins: BTreeSet::new(),
            memory_bytes: 1 << 20,
            gpu_bytes: 0,
        },
        assets: BTreeSet::from(["ui/terminal.json".to_owned()]),
        templates: BTreeSet::from(["ui/terminal.json".to_owned()]),
        channels: Vec::new(),
        actions: BTreeSet::from(["demo.pick".to_owned()]),
    }
}

fn text(value: &str) -> screen::Value {
    screen::Value::Text(value.to_owned())
}

#[test]
fn component_built_against_1_0_still_links_and_skips_newer_events() {
    let mut host = BundleHost::launch(
        include_bytes!("guest_1_0.wat"),
        owner(),
        screen_capabilities(&[Permission::Ui, Permission::Input]),
        1,
    )
    .unwrap();
    let init = host.take_transaction();
    assert!(matches!(
        init.commands.as_slice(),
        [Command::Widget { id, text }] if id == "status" && text == "ready"
    ));
    let message = Event::Message {
        channel: "demo.items".into(),
        record: b"[]".to_vec(),
    };
    assert!(host.dispatch(&message, 1).unwrap().commands.is_empty());
    let epoch = host.dispatch(&Event::Epoch, 2).unwrap();
    assert_eq!(epoch.epoch, 2);
    assert!(epoch.commands.is_empty());
    let action = Event::Action {
        id: "demo.pick".into(),
        index: Some(1),
    };
    assert!(host.dispatch(&action, 2).unwrap().commands.is_empty());
}

#[test]
fn modal_calls_and_callbacks_cross_the_1_1_component_boundary() {
    let mut host = BundleHost::launch(
        guest_1_1("ui/terminal.json").as_bytes(),
        owner(),
        screen_capabilities(&[Permission::ModalUi, Permission::Input]),
        1,
    )
    .unwrap();
    assert!(matches!(
        host.take_transaction().commands.as_slice(),
        [Command::Screen { template: Some(template) }] if template == "ui/terminal.json"
    ));
    let rows = br##"[{"#name":{"type":"text","value":"Stone"}}]"##.to_vec();
    let message = Event::Message {
        channel: "demo.items".into(),
        record: rows,
    };
    match host.dispatch(&message, 1).unwrap().commands.as_slice() {
        [Command::Collection { name, rows }] => {
            assert_eq!(name, "items");
            assert_eq!(rows[0]["#name"], text("Stone"));
        }
        other => panic!("{other:?}"),
    }
    let pick = Event::Action {
        id: "demo.pick".into(),
        index: Some(3),
    };
    assert!(matches!(
        host.dispatch(&pick, 1).unwrap().commands.as_slice(),
        [Command::Value { name, value: screen::Value::Integer(3) }] if name == "#row"
    ));
    let undeclared = Event::Action {
        id: "demo.other".into(),
        index: None,
    };
    assert!(host.dispatch(&undeclared, 1).is_err());
    let epoch = host.dispatch(&Event::Epoch, 5).unwrap();
    assert_eq!(epoch.epoch, 5);
    assert!(matches!(
        epoch.commands.as_slice(),
        [Command::Screen { template: None }]
    ));
    let malformed = Event::Message {
        channel: "demo.items".into(),
        record: b"[{\"name\":1}]".to_vec(),
    };
    assert!(host.dispatch(&malformed, 5).unwrap().commands.is_empty());
}

#[test]
fn modal_screens_need_their_grants_and_an_indexed_template() {
    let mut unindexed = BundleHost::launch(
        guest_1_1("ui/other.json").as_bytes(),
        owner(),
        screen_capabilities(&[Permission::ModalUi, Permission::Input]),
        1,
    )
    .unwrap();
    assert!(unindexed.take_transaction().commands.is_empty());
    let mut denied = BundleHost::launch(
        guest_1_1("ui/terminal.json").as_bytes(),
        owner(),
        screen_capabilities(&[Permission::Ui]),
        1,
    )
    .unwrap();
    assert!(denied.take_transaction().commands.is_empty());
    let pick = Event::Action {
        id: "demo.pick".into(),
        index: None,
    };
    assert!(denied.dispatch(&pick, 1).is_err());
}
