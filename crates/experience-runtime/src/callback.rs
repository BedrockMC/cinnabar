//! One callback: a fresh instance of the guest runs one export against the request's snapshot.
//! What the guest stages through its borrowed `callback` becomes the outcome, or nothing does.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use wasmtime::component::Resource;
use wasmtime::{Engine, Store, Trap};

use crate::hex::{self, HexError};
use crate::host::cinnabar::experience_server::types::{
    self as wit, CallbackInfo, ChangeCause, GuestError, WorldError,
};
use crate::host::{HostState, LimitExceeded};
use crate::limits::{
    CALLBACK_DEADLINE, CALLBACK_FUEL, MAX_BLOCK_DATA_BYTES, MAX_HOST_CALLS, MAX_REASON_BYTES,
    MAX_STAGED_DATA_BYTES, MAX_STAGED_OPS, MAX_TELL_BYTES, MAX_TELLS,
};
use crate::load::Loaded;
use crate::protocol::{self, BlockPos, Call, FailKind, Op, Outcome, Request};

/// The one block id outside its own namespace that an Experience may place.
const AIR: &str = "minecraft:air";
/// Starts a Minecraft formatting code, so tells may not contain it.
const FORMATTING_PREFIX: char = '§';

/// Runs `request` on a fresh instance of `loaded` under the callback fuel, deadline and store
/// limits. The guest sees the snapshot through a borrowed `callback`, and its staged ops commit
/// only if the export returns `ok`. A request that is not a well-formed callback runs nothing
/// and is rejected.
pub fn run(engine: &Engine, loaded: &Loaded, request: &Request) -> Outcome {
    run_metered(engine, loaded, request).0
}

/// [`run`], which also returns the fuel that the callback consumed.
pub fn run_metered(engine: &Engine, loaded: &Loaded, request: &Request) -> (Outcome, u64) {
    let (res, export) = match prepare(&loaded.block_ids, request) {
        Ok(prepared) => prepared,
        Err(reason) => {
            let reason = format!("malformed callback request: {reason}");
            return (Outcome::Rejected { reason }, 0);
        }
    };
    let id = &loaded.manifest.id;
    let mut store = match HostState::store(engine, id, CALLBACK_FUEL, CALLBACK_DEADLINE) {
        Ok(store) => store,
        Err(error) => return (failed(&error), 0),
    };
    let outcome = match invoke(&mut store, loaded, res, &export) {
        Ok((Ok(()), ops)) => Outcome::Committed { ops },
        Ok((Err(GuestError::Rejected(reason) | GuestError::Failed(reason)), _)) => {
            Outcome::Rejected {
                reason: bounded_reason(reason),
            }
        }
        Err(error) => failed(&error),
    };
    // The store meters fuel, so it always reports what is left.
    let fuel = store.get_fuel().map_or(0, |left| CALLBACK_FUEL - left);
    (outcome, fuel)
}

/// A guest's `reason` cut to [`MAX_REASON_BYTES`] at a char boundary, so the result fits in a
/// frame.
fn bounded_reason(mut reason: String) -> String {
    reason.truncate(reason.floor_char_boundary(MAX_REASON_BYTES));
    reason
}

/// Instantiates the guest in `store`, lends it `res` for one export call, and returns the
/// export's result with the ops `res` staged. An error is a trap, or a failure to start.
fn invoke(
    store: &mut Store<HostState>,
    loaded: &Loaded,
    res: CallbackRes,
    export: &Export<'_>,
) -> Result<(Result<(), GuestError>, Vec<Op>)> {
    let server = loaded.pre.instantiate(&mut *store)?;
    let owned = store.data_mut().table.push(res)?;
    // The guest only borrows the callback, for this call; the host keeps `owned`.
    let ctx = Resource::new_borrow(owned.rep());
    let result = match export {
        Export::Place(change) => server.call_on_place(&mut *store, ctx, change),
        Export::Break(change) => server.call_on_break(&mut *store, ctx, change),
        Export::Interact { player, pos, face } => {
            server.call_on_interact(&mut *store, ctx, player, *pos, *face)
        }
        Export::Neighbor { pos, neighbor } => {
            server.call_on_neighbor_changed(&mut *store, ctx, *pos, *neighbor)
        }
    }?;
    let res = store.data_mut().table.delete(owned)?;
    Ok((result, res.ops))
}

/// The outcome of a callback that trapped or could not start: fuel, the deadline and limits
/// each have their own kind, and anything else is a trap. The reason is the root cause alone,
/// because the wasm backtrace around it grows with the guest's stack.
fn failed(error: &anyhow::Error) -> Outcome {
    let kind = match error.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => FailKind::Fuel,
        Some(Trap::Interrupt) => FailKind::Deadline,
        _ if error.is::<LimitExceeded>() => FailKind::Limit,
        _ => FailKind::Trap,
    };
    Outcome::Failed {
        kind,
        reason: error.root_cause().to_string(),
    }
}

/// The host value behind the guest's `callback` handle: what one callback may see, what it has
/// staged, and how much of its caps it has used.
pub struct CallbackRes {
    info: CallbackInfo,
    actor: Option<String>,
    /// The ids of this Experience's blocks.
    own: Arc<[String]>,
    snapshot: Snapshot,
    /// In the order they commit. A position has at most one `SetBlockData`, and it comes after
    /// any `SetBlock` there.
    ops: Vec<Op>,
    /// Bytes this Experience may still add to its block data.
    budget: u64,
    /// Bytes the staged writes add to this Experience's block data; negative when they free
    /// more than they add.
    added: i64,
    /// Bytes of data in the staged `SetBlockData` ops.
    staged_data: usize,
    host_calls: usize,
    tells: usize,
}

/// The snapshot cells by position, with staged writes applied.
struct Snapshot {
    cells: HashMap<BlockPos, Slot>,
    min_y: i32,
    max_y: i32,
    /// The anchor's chunk column; writes stay inside it.
    column: (i32, i32),
}

/// One snapshot cell. `owned` means it holds this Experience's block, which alone has data.
struct Slot {
    loaded: bool,
    id: String,
    owned: bool,
    data: Option<Vec<u8>>,
}

/// The export a callback calls, with its arguments.
enum Export<'a> {
    Place(wit::BlockChange),
    Break(wit::BlockChange),
    Interact {
        player: &'a wit::PlayerId,
        pos: wit::BlockPos,
        face: wit::Face,
    },
    Neighbor {
        pos: wit::BlockPos,
        neighbor: wit::BlockPos,
    },
}

/// The callback's host value and export for `request`, an Experience whose block ids are `own`.
/// The anchor, whose chunk column bounds writes, is the call's position. Hex is decoded and player
/// ids are checked here, so a request that is not a callback, holds bad hex or a player id that
/// is not canonical fails before anything runs.
fn prepare<'a>(
    own: &Arc<[String]>,
    request: &'a Request,
) -> Result<(CallbackRes, Export<'a>), String> {
    let Request::Callback {
        info,
        actor,
        world_min_y,
        world_max_y,
        data_budget,
        snapshot,
        call,
        ..
    } = request
    else {
        return Err("not a callback".to_owned());
    };
    player_id("actor", actor.as_deref())?;
    let (anchor, export) = match call {
        Call::Place { change } => (change.pos, Export::Place(block_change(change)?)),
        Call::Break { change } => (change.pos, Export::Break(block_change(change)?)),
        Call::Interact { player, pos, face } => {
            player_id("player", Some(player))?;
            let export = Export::Interact {
                player,
                pos: (*pos).into(),
                face: (*face).into(),
            };
            (*pos, export)
        }
        Call::Neighbor { pos, neighbor } => (
            *pos,
            Export::Neighbor {
                pos: (*pos).into(),
                neighbor: (*neighbor).into(),
            },
        ),
    };
    let cells = snapshot
        .iter()
        .map(|cell| {
            let data = decode(cell.data.as_deref())
                .map_err(|error| format!("data of cell {:?}: {error}", cell.pos))?;
            let slot = Slot {
                loaded: cell.loaded,
                id: cell.id.clone(),
                owned: cell.owned,
                data,
            };
            Ok((cell.pos, slot))
        })
        .collect::<Result<_, String>>()?;
    let res = CallbackRes {
        info: CallbackInfo {
            world_id: info.world_id.clone(),
            dimension_id: info.dimension_id.clone(),
            tick: info.tick,
            event_sequence: info.event_sequence,
        },
        actor: actor.clone(),
        own: Arc::clone(own),
        snapshot: Snapshot {
            cells,
            min_y: *world_min_y,
            max_y: *world_max_y,
            column: column(anchor),
        },
        ops: Vec::new(),
        budget: *data_budget,
        added: 0,
        staged_data: 0,
        host_calls: 0,
        tells: 0,
    };
    Ok((res, export))
}

fn block_change(change: &protocol::Change) -> Result<wit::BlockChange, String> {
    player_id("change actor", change.actor.as_deref())?;
    let previous_data = decode(change.previous_data.as_deref())
        .map_err(|error| format!("previous data: {error}"))?;
    Ok(wit::BlockChange {
        pos: change.pos.into(),
        actor: change.actor.clone(),
        cause: change.cause.into(),
        before_id: change.before_id.clone(),
        after_id: change.after_id.clone(),
        previous_data,
    })
}

/// Refuses `id`, named `what`, when it is present but not a canonical player id.
fn player_id(what: &str, id: Option<&str>) -> Result<(), String> {
    match id {
        Some(id) if !protocol::is_player_id(id) => Err(format!(
            "{what} is not a canonical lowercase hyphenated UUID"
        )),
        _ => Ok(()),
    }
}

fn decode(data: Option<&str>) -> Result<Option<Vec<u8>>, HexError> {
    data.map(hex::decode).transpose()
}

/// The 16×16 chunk column holding `pos`; the arithmetic shift rounds negative coordinates down.
fn column(pos: BlockPos) -> (i32, i32) {
    (pos.x >> 4, pos.z >> 4)
}

impl Snapshot {
    /// A loaded snapshot cell within the world height, which the guest may read.
    fn read(&self, pos: BlockPos) -> Result<&Slot, WorldError> {
        self.within_height(pos)?;
        let slot = self.cells.get(&pos).ok_or(WorldError::Denied)?;
        if slot.loaded {
            Ok(slot)
        } else {
            Err(WorldError::Unavailable)
        }
    }

    /// A cell the guest may read that is also in the anchor's chunk column, so it may write it.
    fn write(&mut self, pos: BlockPos) -> Result<&mut Slot, WorldError> {
        self.within_height(pos)?;
        if column(pos) != self.column {
            return Err(WorldError::Denied);
        }
        let slot = self.cells.get_mut(&pos).ok_or(WorldError::Denied)?;
        if slot.loaded {
            Ok(slot)
        } else {
            Err(WorldError::Unavailable)
        }
    }

    fn within_height(&self, pos: BlockPos) -> Result<(), WorldError> {
        if (self.min_y..=self.max_y).contains(&pos.y) {
            Ok(())
        } else {
            Err(WorldError::OutOfBounds)
        }
    }
}

/// The world-access methods. Each counts as a host call; the outer error is a trap, and the
/// inner one is a refusal that leaves everything staged as it was.
impl CallbackRes {
    pub(crate) fn info(&mut self) -> Result<CallbackInfo> {
        self.host_call()?;
        Ok(self.info.clone())
    }

    pub(crate) fn get_block(&mut self, pos: BlockPos) -> Result<Result<String, WorldError>> {
        self.host_call()?;
        Ok(self.snapshot.read(pos).map(|slot| slot.id.clone()))
    }

    /// Replaces air or an own block with air or an own block. The position loses its data, so
    /// data staged for it is dropped, and it is owned exactly when the new block is this
    /// Experience's.
    pub(crate) fn set_block(
        &mut self,
        pos: BlockPos,
        id: String,
    ) -> Result<Result<(), WorldError>> {
        self.host_call()?;
        let slot = match self.snapshot.write(pos) {
            Ok(slot) => slot,
            Err(error) => return Ok(Err(error)),
        };
        if slot.id != AIR && !slot.owned {
            return Ok(Err(WorldError::NotOwned));
        }
        let owned = id != AIR;
        if owned && !self.own.contains(&id) {
            return Ok(Err(WorldError::UnknownBlock));
        }
        if let Some(index) = data_op(&self.ops, pos) {
            self.staged_data -= staged_len(&self.ops.remove(index));
        }
        stage(
            &mut self.ops,
            Op::SetBlock {
                pos,
                id: id.clone(),
            },
        )?;
        self.added -= len(slot.data.as_deref());
        slot.id = id;
        slot.owned = owned;
        slot.data = None;
        Ok(Ok(()))
    }

    pub(crate) fn block_data(
        &mut self,
        pos: BlockPos,
    ) -> Result<Result<Option<Vec<u8>>, WorldError>> {
        self.host_call()?;
        Ok(self.snapshot.read(pos).and_then(|slot| {
            if slot.owned {
                Ok(slot.data.clone())
            } else {
                Err(WorldError::NotOwned)
            }
        }))
    }

    /// Writes or, with `None`, deletes an own block's data, within the size limits and the data
    /// budget. A rewrite replaces the op staged for the block in place.
    pub(crate) fn set_block_data(
        &mut self,
        pos: BlockPos,
        data: Option<Vec<u8>>,
    ) -> Result<Result<(), WorldError>> {
        self.host_call()?;
        let slot = match self.snapshot.write(pos) {
            Ok(slot) => slot,
            Err(error) => return Ok(Err(error)),
        };
        if !slot.owned {
            return Ok(Err(WorldError::NotOwned));
        }
        let size = data.as_ref().map_or(0, Vec::len);
        let earlier = data_op(&self.ops, pos);
        let staged =
            self.staged_data + size - earlier.map_or(0, |index| staged_len(&self.ops[index]));
        if size > MAX_BLOCK_DATA_BYTES || staged > MAX_STAGED_DATA_BYTES {
            return Ok(Err(WorldError::TooLarge));
        }
        let added = self.added + len(data.as_deref()) - len(slot.data.as_deref());
        if u64::try_from(added).is_ok_and(|added| added > self.budget) {
            return Ok(Err(WorldError::QuotaExceeded));
        }
        let op = Op::SetBlockData {
            pos,
            data: data.as_deref().map(hex::encode),
        };
        match earlier {
            Some(index) => self.ops[index] = op,
            None => stage(&mut self.ops, op)?,
        }
        self.staged_data = staged;
        self.added = added;
        slot.data = data;
        Ok(Ok(()))
    }

    /// Tells the event's actor `text`; the tell past [`MAX_TELLS`] traps.
    pub(crate) fn tell(&mut self, player: String, text: String) -> Result<Result<(), WorldError>> {
        self.host_call()?;
        match &self.actor {
            None => return Ok(Err(WorldError::PlayerUnavailable)),
            Some(actor) if *actor != player => return Ok(Err(WorldError::Denied)),
            Some(_) => {}
        }
        if text.len() > MAX_TELL_BYTES {
            return Ok(Err(WorldError::TooLarge));
        }
        if text
            .chars()
            .any(|c| c.is_control() || c == FORMATTING_PREFIX)
        {
            return Ok(Err(WorldError::InvalidText));
        }
        if self.tells == MAX_TELLS {
            return Err(LimitExceeded(format!("more than {MAX_TELLS} tells")).into());
        }
        stage(&mut self.ops, Op::Tell { player, text })?;
        self.tells += 1;
        Ok(Ok(()))
    }

    /// Counts one host call; the call past [`MAX_HOST_CALLS`] traps.
    fn host_call(&mut self) -> Result<()> {
        if self.host_calls == MAX_HOST_CALLS {
            return Err(LimitExceeded(format!("more than {MAX_HOST_CALLS} host calls")).into());
        }
        self.host_calls += 1;
        Ok(())
    }
}

/// Appends `op`; the op past [`MAX_STAGED_OPS`] traps.
fn stage(ops: &mut Vec<Op>, op: Op) -> Result<()> {
    if ops.len() == MAX_STAGED_OPS {
        return Err(LimitExceeded(format!("more than {MAX_STAGED_OPS} staged ops")).into());
    }
    ops.push(op);
    Ok(())
}

/// The index of the staged `SetBlockData` at `pos`.
fn data_op(ops: &[Op], pos: BlockPos) -> Option<usize> {
    ops.iter()
        .position(|op| matches!(op, Op::SetBlockData { pos: at, .. } if *at == pos))
}

/// The bytes of data that a staged op writes; its hex has two digits per byte.
fn staged_len(op: &Op) -> usize {
    match op {
        Op::SetBlockData {
            data: Some(hex), ..
        } => hex.len() / 2,
        _ => 0,
    }
}

/// The length of some data; absent data has none. A slice holds at most `isize::MAX` bytes, so
/// the length fits.
fn len(data: Option<&[u8]>) -> i64 {
    data.map_or(0, |data| data.len() as i64)
}

impl From<BlockPos> for wit::BlockPos {
    fn from(BlockPos { x, y, z }: BlockPos) -> Self {
        Self { x, y, z }
    }
}

impl From<wit::BlockPos> for BlockPos {
    fn from(wit::BlockPos { x, y, z }: wit::BlockPos) -> Self {
        Self { x, y, z }
    }
}

impl From<protocol::Face> for wit::Face {
    fn from(face: protocol::Face) -> Self {
        match face {
            protocol::Face::Down => Self::Down,
            protocol::Face::Up => Self::Up,
            protocol::Face::North => Self::North,
            protocol::Face::South => Self::South,
            protocol::Face::West => Self::West,
            protocol::Face::East => Self::East,
        }
    }
}

impl From<protocol::Cause> for ChangeCause {
    fn from(cause: protocol::Cause) -> Self {
        match cause {
            protocol::Cause::Player => Self::Player,
            protocol::Cause::Guest => Self::Guest,
            protocol::Cause::Environment => Self::Environment,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use wasmtime::Trap;

    use super::{CallbackRes, failed, prepare};
    use crate::host::LimitExceeded;
    use crate::host::cinnabar::experience_server::types::WorldError;
    use crate::limits::{
        MAX_BLOCK_DATA_BYTES, MAX_HOST_CALLS, MAX_STAGED_DATA_BYTES, MAX_STAGED_OPS,
        MAX_TELL_BYTES, MAX_TELLS,
    };
    use crate::protocol::{BlockPos, Call, Cell, Face, FailKind, Info, Op, Outcome, Request};

    const ACTOR: &str = "3f2a7c1e-8b4d-4e6a-9c5f-1d2e3f4a5b6c";
    const COUNTER: &str = "probe:counter";
    const AIR: &str = "minecraft:air";

    const ANCHOR: BlockPos = BlockPos { x: 0, y: 64, z: 0 };
    const UP: BlockPos = BlockPos { x: 0, y: 65, z: 0 };
    const DOWN: BlockPos = BlockPos { x: 0, y: 63, z: 0 };
    const EAST: BlockPos = BlockPos { x: 1, y: 64, z: 0 };
    const WEST: BlockPos = BlockPos { x: -1, y: 64, z: 0 };
    const NORTH: BlockPos = BlockPos { x: 0, y: 64, z: -1 };
    const SOUTH: BlockPos = BlockPos { x: 0, y: 64, z: 1 };

    /// The actor's interaction with [`ANCHOR`], an owned probe:counter without data whose six
    /// neighbors are loaded air, with room to spare in the world and the data budget.
    struct Fixture {
        actor: Option<&'static str>,
        min_y: i32,
        max_y: i32,
        budget: u64,
        cells: Vec<Cell>,
    }

    impl Fixture {
        fn new() -> Self {
            let mut cells = vec![loaded(ANCHOR, COUNTER, true, None)];
            for pos in [UP, DOWN, EAST, WEST, NORTH, SOUTH] {
                cells.push(loaded(pos, AIR, false, None));
            }
            Self {
                actor: Some(ACTOR),
                min_y: -64,
                max_y: 319,
                budget: 1 << 20,
                cells,
            }
        }

        /// Replaces the cell at `pos` with a loaded `id`; `data` is hex.
        fn cell(mut self, pos: BlockPos, id: &str, owned: bool, data: Option<&str>) -> Self {
            let cell = self.cells.iter_mut().find(|cell| cell.pos == pos);
            *cell.expect("a snapshot cell") = loaded(pos, id, owned, data);
            self
        }

        /// The callback's host value, for an Experience whose only block is probe:counter.
        fn res(self) -> CallbackRes {
            let request = Request::Callback {
                seq: 1,
                info: Info {
                    world_id: "world".to_owned(),
                    dimension_id: "overworld".to_owned(),
                    tick: 1,
                    event_sequence: 1,
                },
                actor: self.actor.map(str::to_owned),
                world_min_y: self.min_y,
                world_max_y: self.max_y,
                data_budget: self.budget,
                snapshot: self.cells,
                call: Call::Interact {
                    player: ACTOR.to_owned(),
                    pos: ANCHOR,
                    face: Face::Up,
                },
            };
            let (res, _) = prepare(&Arc::from([COUNTER.to_owned()]), &request).unwrap();
            res
        }
    }

    fn loaded(pos: BlockPos, id: &str, owned: bool, data: Option<&str>) -> Cell {
        Cell {
            pos,
            loaded: true,
            id: id.to_owned(),
            owned,
            data: data.map(str::to_owned),
        }
    }

    /// Whether `result` is a trap that fails the callback as `limit`.
    fn limited<T>(result: anyhow::Result<T>) -> bool {
        result.is_err_and(|error| error.is::<LimitExceeded>())
    }

    /// Both bounds are inside the world; the cells past them are outside even though the
    /// snapshot holds them.
    #[test]
    fn world_height_bounds_reads_and_writes() {
        let mut res = Fixture {
            min_y: ANCHOR.y,
            max_y: ANCHOR.y,
            ..Fixture::new()
        }
        .res();
        assert_eq!(res.get_block(ANCHOR).unwrap(), Ok(COUNTER.to_owned()));
        assert_eq!(res.get_block(UP).unwrap(), Err(WorldError::OutOfBounds));
        assert_eq!(res.block_data(DOWN).unwrap(), Err(WorldError::OutOfBounds));
        assert_eq!(
            res.set_block(UP, AIR.to_owned()).unwrap(),
            Err(WorldError::OutOfBounds)
        );
        assert_eq!(
            res.set_block_data(DOWN, None).unwrap(),
            Err(WorldError::OutOfBounds)
        );
    }

    /// West and north of the anchor lie in other chunk columns (`-1 >> 4 == -1`); east shares
    /// the anchor's. Reads reach all of them.
    #[test]
    fn writes_stay_in_the_anchor_chunk_column() {
        let mut res = Fixture::new().cell(NORTH, COUNTER, true, None).res();
        assert_eq!(res.get_block(WEST).unwrap(), Ok(AIR.to_owned()));
        assert_eq!(
            res.set_block(WEST, COUNTER.to_owned()).unwrap(),
            Err(WorldError::Denied)
        );
        assert_eq!(res.block_data(NORTH).unwrap(), Ok(None));
        assert_eq!(
            res.set_block_data(NORTH, Some(vec![1])).unwrap(),
            Err(WorldError::Denied)
        );
        assert_eq!(res.set_block(EAST, COUNTER.to_owned()).unwrap(), Ok(()));
    }

    /// Placing makes air an owned block without data; removing makes it air that is not owned.
    /// Refused replacements stage nothing.
    #[test]
    fn set_block_replaces_air_or_own_blocks_with_known_ids() {
        let mut res = Fixture::new()
            .cell(EAST, "minecraft:stone", false, None)
            .res();
        assert_eq!(
            res.set_block(EAST, AIR.to_owned()).unwrap(),
            Err(WorldError::NotOwned)
        );
        assert_eq!(
            res.set_block(UP, "probe:missing".to_owned()).unwrap(),
            Err(WorldError::UnknownBlock)
        );
        assert_eq!(res.set_block(UP, COUNTER.to_owned()).unwrap(), Ok(()));
        assert_eq!(res.block_data(UP).unwrap(), Ok(None));
        assert_eq!(res.set_block(ANCHOR, AIR.to_owned()).unwrap(), Ok(()));
        assert_eq!(res.block_data(ANCHOR).unwrap(), Err(WorldError::NotOwned));
        assert_eq!(
            res.ops,
            vec![
                Op::SetBlock {
                    pos: UP,
                    id: COUNTER.to_owned(),
                },
                Op::SetBlock {
                    pos: ANCHOR,
                    id: AIR.to_owned(),
                },
            ]
        );
    }

    #[test]
    fn data_belongs_to_own_blocks_only() {
        let mut res = Fixture::new().res();
        assert_eq!(res.block_data(UP).unwrap(), Err(WorldError::NotOwned));
        assert_eq!(
            res.set_block_data(UP, Some(vec![1])).unwrap(),
            Err(WorldError::NotOwned)
        );
    }

    /// The limit itself fits; one byte more is refused and leaves the data as it was.
    #[test]
    fn block_data_limit_is_inclusive() {
        let mut res = Fixture::new().res();
        let full = vec![7; MAX_BLOCK_DATA_BYTES];
        assert_eq!(
            res.set_block_data(ANCHOR, Some(full.clone())).unwrap(),
            Ok(())
        );
        assert_eq!(
            res.set_block_data(ANCHOR, Some(vec![0; MAX_BLOCK_DATA_BYTES + 1]))
                .unwrap(),
            Err(WorldError::TooLarge)
        );
        assert_eq!(res.block_data(ANCHOR).unwrap(), Ok(Some(full)));
    }

    /// The budget bounds the bytes a callback adds in total, so shrinking or clearing data
    /// frees room for later writes.
    #[test]
    fn quota_counts_net_growth() {
        let mut res = Fixture {
            budget: 3,
            ..Fixture::new()
        }
        .cell(ANCHOR, COUNTER, true, Some("0102"))
        .cell(EAST, COUNTER, true, None)
        .res();
        // Growing 2 bytes to 5 adds exactly the budget.
        assert_eq!(
            res.set_block_data(ANCHOR, Some(vec![0; 5])).unwrap(),
            Ok(())
        );
        assert_eq!(
            res.set_block_data(EAST, Some(vec![0])).unwrap(),
            Err(WorldError::QuotaExceeded)
        );
        // Replacing the anchor clears its 5 bytes.
        assert_eq!(res.set_block(ANCHOR, COUNTER.to_owned()).unwrap(), Ok(()));
        assert_eq!(res.set_block_data(EAST, Some(vec![0; 5])).unwrap(), Ok(()));
        assert_eq!(
            res.set_block_data(EAST, Some(vec![0; 6])).unwrap(),
            Err(WorldError::QuotaExceeded)
        );
    }

    /// A rewrite replaces the op it rewrites, so it neither grows the result nor counts against
    /// the op cap, and the last write is the one staged.
    #[test]
    fn rewriting_data_stages_one_op() {
        let mut res = Fixture::new().res();
        for _ in 0..MAX_STAGED_OPS {
            assert_eq!(res.set_block_data(ANCHOR, Some(vec![0])).unwrap(), Ok(()));
        }
        assert_eq!(
            res.set_block_data(ANCHOR, Some(vec![0xab])).unwrap(),
            Ok(())
        );
        assert_eq!(
            res.ops,
            vec![Op::SetBlockData {
                pos: ANCHOR,
                data: Some("ab".to_owned()),
            }]
        );
    }

    /// The replacement clears the data staged before it, so that op is dropped. The ops are
    /// applied in order, so data staged after the replacement must stay after it.
    #[test]
    fn replacement_drops_data_staged_before_it() {
        let mut res = Fixture::new().res();
        assert_eq!(res.set_block_data(ANCHOR, Some(vec![1])).unwrap(), Ok(()));
        assert_eq!(res.set_block(ANCHOR, COUNTER.to_owned()).unwrap(), Ok(()));
        assert_eq!(res.set_block_data(ANCHOR, Some(vec![2])).unwrap(), Ok(()));
        assert_eq!(
            res.ops,
            vec![
                Op::SetBlock {
                    pos: ANCHOR,
                    id: COUNTER.to_owned(),
                },
                Op::SetBlockData {
                    pos: ANCHOR,
                    data: Some("02".to_owned()),
                },
            ]
        );
    }

    /// Owned cells in the anchor's chunk column whose full data fills the staged-data limit.
    const FULL: [BlockPos; 4] = [ANCHOR, UP, DOWN, EAST];

    /// A callback that has staged [`MAX_BLOCK_DATA_BYTES`] in every cell of [`FULL`], which
    /// reaches the staged-data limit exactly. [`SOUTH`] is owned too and has no data.
    fn full_staged_data() -> CallbackRes {
        assert_eq!(
            FULL.len() * MAX_BLOCK_DATA_BYTES,
            MAX_STAGED_DATA_BYTES,
            "FULL must fill the staged-data limit exactly"
        );
        let mut res = Fixture::new()
            .cell(UP, COUNTER, true, None)
            .cell(DOWN, COUNTER, true, None)
            .cell(EAST, COUNTER, true, None)
            .cell(SOUTH, COUNTER, true, None)
            .res();
        for pos in FULL {
            let full = Some(vec![0; MAX_BLOCK_DATA_BYTES]);
            assert_eq!(res.set_block_data(pos, full).unwrap(), Ok(()));
        }
        res
    }

    /// The limit itself fits; a byte more is refused and stages nothing.
    #[test]
    fn staged_data_limit_is_inclusive() {
        let mut res = full_staged_data();
        assert_eq!(
            res.set_block_data(SOUTH, Some(vec![0])).unwrap(),
            Err(WorldError::TooLarge)
        );
        assert_eq!(res.block_data(SOUTH).unwrap(), Ok(None));
        assert_eq!(res.ops.len(), FULL.len());
    }

    /// Only the ops left staged count, so shrinking a rewrite and replacing a block both free
    /// room.
    #[test]
    fn staged_data_counts_the_ops_left_after_rewrites() {
        let mut res = full_staged_data();
        let shrunk = Some(vec![0; MAX_BLOCK_DATA_BYTES - 1]);
        assert_eq!(res.set_block_data(ANCHOR, shrunk).unwrap(), Ok(()));
        assert_eq!(res.set_block_data(SOUTH, Some(vec![0])).unwrap(), Ok(()));
        assert_eq!(res.set_block(UP, COUNTER.to_owned()).unwrap(), Ok(()));
        let full = Some(vec![0; MAX_BLOCK_DATA_BYTES]);
        assert_eq!(res.set_block_data(SOUTH, full).unwrap(), Ok(()));
    }

    #[test]
    fn tell_reaches_only_the_actor() {
        let mut res = Fixture::new().res();
        let stranger = "00000000-0000-0000-0000-000000000000".to_owned();
        assert_eq!(
            res.tell(stranger, "x".to_owned()).unwrap(),
            Err(WorldError::Denied)
        );
        let mut res = Fixture {
            actor: None,
            ..Fixture::new()
        }
        .res();
        assert_eq!(
            res.tell(ACTOR.to_owned(), "x".to_owned()).unwrap(),
            Err(WorldError::PlayerUnavailable)
        );
    }

    /// The size limit counts UTF-8 bytes, so 129 two-byte characters are too many.
    #[test]
    fn tell_text_is_plain_and_short() {
        let mut res = Fixture::new().res();
        let mut tell = |text: String| res.tell(ACTOR.to_owned(), text).unwrap();
        assert_eq!(tell("a\nb".to_owned()), Err(WorldError::InvalidText));
        assert_eq!(tell("§cred".to_owned()), Err(WorldError::InvalidText));
        assert_eq!(
            tell("a".repeat(MAX_TELL_BYTES + 1)),
            Err(WorldError::TooLarge)
        );
        assert_eq!(
            tell("é".repeat(MAX_TELL_BYTES / 2 + 1)),
            Err(WorldError::TooLarge)
        );
        assert_eq!(tell("a".repeat(MAX_TELL_BYTES)), Ok(()));
        assert_eq!(
            res.ops,
            vec![Op::Tell {
                player: ACTOR.to_owned(),
                text: "a".repeat(MAX_TELL_BYTES),
            }]
        );
    }

    #[test]
    fn tell_past_the_cap_traps() {
        let mut res = Fixture::new().res();
        for _ in 0..MAX_TELLS {
            assert_eq!(res.tell(ACTOR.to_owned(), "x".to_owned()).unwrap(), Ok(()));
        }
        assert!(limited(res.tell(ACTOR.to_owned(), "x".to_owned())));
    }

    #[test]
    fn op_past_the_cap_traps() {
        let mut res = Fixture::new().res();
        for _ in 0..MAX_STAGED_OPS {
            assert_eq!(res.set_block(UP, AIR.to_owned()).unwrap(), Ok(()));
        }
        assert!(limited(res.set_block(UP, AIR.to_owned())));
    }

    /// `info` is a host call too.
    #[test]
    fn host_call_past_the_cap_traps() {
        let mut res = Fixture::new().res();
        for _ in 0..MAX_HOST_CALLS {
            assert_eq!(res.get_block(ANCHOR).unwrap(), Ok(COUNTER.to_owned()));
        }
        assert!(limited(res.info()));
    }

    /// A guest trap arrives wrapped in its backtrace, which the reason leaves out.
    #[test]
    fn failures_are_classified_by_cause() {
        let cases: [(anyhow::Error, FailKind); 4] = [
            (Trap::OutOfFuel.into(), FailKind::Fuel),
            (Trap::Interrupt.into(), FailKind::Deadline),
            (LimitExceeded("too many".to_owned()).into(), FailKind::Limit),
            (Trap::UnreachableCodeReached.into(), FailKind::Trap),
        ];
        for (cause, kind) in cases {
            let reason = cause.to_string();
            let error = cause.context("error while executing at wasm backtrace: …");
            assert_eq!(failed(&error), Outcome::Failed { kind, reason });
        }
    }
}
