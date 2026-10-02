//! Callbacks on the probe guest: what each behavior stages and how each call ends. The probe
//! selects a behavior by the interacted block's x; `p(x)` is that block and `up(x)` the one above.

mod common;

use std::sync::LazyLock;

use common::probe_dir;
use experience_runtime::callback::run;
use experience_runtime::load::{EpochTicker, Loaded, engine, load};
use experience_runtime::protocol::{
    BlockPos, Call, Cause, Cell, Change, Face, FailKind, Info, Op, Outcome, Request,
};
use wasmtime::Engine;

const ACTOR: &str = "3f2a7c1e-8b4d-4e6a-9c5f-1d2e3f4a5b6c";
const COUNTER: &str = "probe:counter";
const AIR: &str = "minecraft:air";

/// The probe, loaded once for every test in this binary.
struct Probe {
    engine: Engine,
    loaded: Loaded,
    _ticker: EpochTicker,
}

fn probe() -> &'static Probe {
    static PROBE: LazyLock<Probe> = LazyLock::new(|| {
        let (engine, ticker) = engine().unwrap();
        // The artifact is only read while loading.
        let dir = probe_dir();
        let loaded = load(&engine, dir.path()).unwrap();
        Probe {
            engine,
            loaded,
            _ticker: ticker,
        }
    });
    &PROBE
}

fn outcome(request: &Request) -> Outcome {
    let probe = probe();
    run(&probe.engine, &probe.loaded, request)
}

fn p(x: i32) -> BlockPos {
    BlockPos { x, y: 64, z: 0 }
}

fn up(x: i32) -> BlockPos {
    BlockPos { x, y: 65, z: 0 }
}

/// A loaded cell; `data` is hex.
fn cell(pos: BlockPos, id: &str, owned: bool, data: Option<&str>) -> Cell {
    Cell {
        pos,
        loaded: true,
        id: id.to_owned(),
        owned,
        data: data.map(str::to_owned),
    }
}

/// A callback from the actor for `call` at `anchor`, with a 7-cell snapshot: the anchor is an
/// owned probe:counter without data and its six neighbors are loaded air. The world height and the
/// data budget leave room.
fn callback(anchor: BlockPos, call: Call) -> Request {
    let BlockPos { x, y, z } = anchor;
    let neighbors = [
        (x + 1, y, z),
        (x - 1, y, z),
        (x, y + 1, z),
        (x, y - 1, z),
        (x, y, z + 1),
        (x, y, z - 1),
    ];
    let mut snapshot = vec![cell(anchor, COUNTER, true, None)];
    snapshot.extend(
        neighbors
            .into_iter()
            .map(|(x, y, z)| cell(BlockPos { x, y, z }, AIR, false, None)),
    );
    Request::Callback {
        seq: 1,
        info: Info {
            world_id: "world".to_owned(),
            dimension_id: "overworld".to_owned(),
            tick: 1,
            event_sequence: 1,
        },
        actor: Some(ACTOR.to_owned()),
        world_min_y: -64,
        world_max_y: 319,
        data_budget: 1 << 20,
        snapshot,
        call,
    }
}

/// The actor's interaction with `p(x)`.
fn interact(x: i32) -> Request {
    callback(
        p(x),
        Call::Interact {
            player: ACTOR.to_owned(),
            pos: p(x),
            face: Face::Up,
        },
    )
}

/// `request` with its snapshot cell at `new.pos` replaced by `new`.
fn with_cell(mut request: Request, new: Cell) -> Request {
    let Request::Callback { snapshot, .. } = &mut request else {
        unreachable!("a callback request");
    };
    let old = snapshot
        .iter_mut()
        .find(|cell| cell.pos == new.pos)
        .expect("the cell is in the snapshot");
    *old = new;
    request
}

fn tell(text: &str) -> Op {
    Op::Tell {
        player: ACTOR.to_owned(),
        text: text.to_owned(),
    }
}

fn committed(ops: Vec<Op>) -> Outcome {
    Outcome::Committed { ops }
}

#[test]
fn counter_commits_data_then_tell() {
    assert_eq!(
        outcome(&interact(0)),
        committed(vec![
            Op::SetBlockData {
                pos: p(0),
                data: Some("01000000".to_owned()),
            },
            tell("count 1"),
        ])
    );
}

#[test]
fn trap_after_staging_commits_nothing() {
    let outcome = outcome(&interact(1));
    assert!(
        matches!(
            outcome,
            Outcome::Failed {
                kind: FailKind::Trap,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn guest_error_commits_nothing() {
    assert_eq!(
        outcome(&interact(10)),
        Outcome::Rejected {
            reason: "nope".to_owned()
        }
    );
}

#[test]
fn reads_see_staged_writes() {
    assert_eq!(
        outcome(&interact(5)),
        committed(vec![
            Op::SetBlock {
                pos: up(5),
                id: COUNTER.to_owned(),
            },
            tell(COUNTER),
        ])
    );
}

#[test]
fn read_outside_snapshot_is_denied() {
    assert_eq!(outcome(&interact(6)), committed(vec![tell("denied")]));
}

/// The probe's `set-block(up)` fails too, so nothing but the tell is staged.
#[test]
fn unloaded_cell_is_unavailable() {
    let unloaded = Cell {
        pos: up(5),
        loaded: false,
        id: String::new(),
        owned: false,
        data: None,
    };
    assert_eq!(
        outcome(&with_cell(interact(5), unloaded)),
        committed(vec![tell("unavailable")])
    );
}

#[test]
fn data_over_cap_is_too_large() {
    assert_eq!(outcome(&interact(7)), committed(vec![tell("too-large")]));
}

#[test]
fn data_over_budget_is_quota_exceeded() {
    let mut request = interact(0);
    let Request::Callback { data_budget, .. } = &mut request else {
        unreachable!("a callback request");
    };
    *data_budget = 0;
    assert_eq!(
        outcome(&request),
        committed(vec![tell("error quota-exceeded")])
    );
}

#[test]
fn vanilla_target_is_unknown_block() {
    assert_eq!(
        outcome(&interact(8)),
        committed(vec![tell("unknown-block")])
    );
}

#[test]
fn tell_to_non_actor_is_denied() {
    assert_eq!(outcome(&interact(9)), committed(vec![tell("denied")]));
}

/// `p` starts with data, so each write must replace it to be read back.
#[test]
fn none_and_empty_data_differ() {
    let request = |x| with_cell(interact(x), cell(p(x), COUNTER, true, Some("0a")));
    assert_eq!(
        outcome(&request(13)),
        committed(vec![
            Op::SetBlockData {
                pos: p(13),
                data: None,
            },
            tell("absent"),
        ])
    );
    assert_eq!(
        outcome(&request(14)),
        committed(vec![
            Op::SetBlockData {
                pos: p(14),
                data: Some(String::new()),
            },
            tell("empty"),
        ])
    );
}

#[test]
fn info_is_passed_through() {
    let mut request = interact(15);
    let Request::Callback { info, .. } = &mut request else {
        unreachable!("a callback request");
    };
    info.tick = 77;
    info.event_sequence = 5;
    assert_eq!(outcome(&request), committed(vec![tell("tick 77 seq 5")]));
}

#[test]
fn place_break_neighbor_reach_guest() {
    let change = |before_id: &str, after_id: &str, previous_data: Option<&str>| Change {
        pos: p(0),
        actor: Some(ACTOR.to_owned()),
        cause: Cause::Player,
        before_id: before_id.to_owned(),
        after_id: after_id.to_owned(),
        previous_data: previous_data.map(str::to_owned),
    };
    let place = callback(
        p(0),
        Call::Place {
            change: change(AIR, COUNTER, None),
        },
    );
    assert_eq!(outcome(&place), committed(vec![tell("placed")]));
    let broken = callback(
        p(0),
        Call::Break {
            change: change(COUNTER, AIR, Some("0a")),
        },
    );
    let broken = with_cell(broken, cell(p(0), AIR, false, None));
    assert_eq!(outcome(&broken), committed(vec![tell("broke 1")]));
    let neighbor = callback(
        p(0),
        Call::Neighbor {
            pos: p(0),
            neighbor: up(0),
        },
    );
    assert_eq!(outcome(&neighbor), committed(vec![]));
}

/// x=17 replaces the owned `up`, which holds data, with the same block, then reads its data.
#[test]
fn replacing_block_clears_its_data_in_overlay() {
    let request = with_cell(interact(17), cell(up(17), COUNTER, true, Some("0a")));
    assert_eq!(
        outcome(&request),
        committed(vec![
            Op::SetBlock {
                pos: up(17),
                id: COUNTER.to_owned(),
            },
            tell("absent"),
        ])
    );
}

/// Snapshot data that is not hex runs nothing: the guest would have committed.
#[test]
fn malformed_request_is_rejected_unrun() {
    let request = with_cell(interact(0), cell(p(0), COUNTER, true, Some("zz")));
    let outcome = outcome(&request);
    assert!(matches!(outcome, Outcome::Rejected { .. }), "{outcome:?}");
}
