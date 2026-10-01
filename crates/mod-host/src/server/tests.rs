use super::*;
use server_experience::manifest::{Permission, Scope};

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
            channels: Vec::new(),
            actions: BTreeSet::new(),
        },
        actions: BTreeSet::new(),
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
    let size = serde_json::to_vec(&command).unwrap().len();
    while MAX_HOST_OUTPUT - state.bytes > size + 1 {
        state.stage(command.clone()).unwrap().unwrap();
    }
    let empty = Command::Widget {
        id: "w".repeat(MAX_IDENTIFIER_BYTES),
        text: String::new(),
    };
    let overhead = serde_json::to_vec(&empty).unwrap().len() + 1;
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
