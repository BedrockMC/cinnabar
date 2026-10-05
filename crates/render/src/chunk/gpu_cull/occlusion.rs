//! Which sub-chunks a direct-draw frame may skip on the strength of read-back occlusion bits.
//!
//! A verdict comes from a depth one or more frames old. Occlusion along a ray depends only on
//! the eye position, so a verdict is honoured only while the eye, projection, depth size and
//! resident geometry match the frame it was computed on; any eye translation voids it, because
//! parallax past a near occluder uncovers far terrain by tens of pixels per block. For the same
//! reason a settled verdict stays true until one of those changes, so a still camera stops
//! asking for new ones.

/// Consecutive occluded readbacks under one basis before a slot is skipped.
pub const REQUIRED_OCCLUDED_READBACKS: u8 = 2;
/// View turn (radians) since the latest verdict beyond which a frame skips nothing.
pub const MAX_VERDICT_TURN: f32 = 0.35;

/// What a verdict's depth was rendered from; a verdict holds only while all of it matches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcclusionBasis {
    pub eye: [f32; 3],
    /// Column-major `clip_from_view`; the near plane decides which occluders rasterise.
    pub clip_from_view: [f32; 16],
    pub depth_size: [u32; 2],
    /// Bumped whenever resident geometry may have uncovered something.
    pub world: u64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OcclusionView {
    pub basis: OcclusionBasis,
    /// Unit view direction.
    pub forward: [f32; 3],
}

/// The frame and view one readback's bits were computed for, over its first `slots` slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VerdictTag {
    pub frame: u64,
    pub view: OcclusionView,
    pub slots: u32,
}

/// Per-slot runs of occluded verdicts under the latest basis.
#[derive(Debug, Default)]
pub struct OcclusionHistory {
    runs: Vec<u8>,
    /// Frame each slot's current record was written; older verdicts describe another record.
    since: Vec<u64>,
    basis: Option<OcclusionBasis>,
    forward: [f32; 3],
    world: u64,
    /// Verdicts since the basis, view direction or any record last changed.
    settled: u8,
    assigned_at: u64,
}

impl OcclusionHistory {
    pub fn world(&self) -> u64 {
        self.world
    }

    /// Geometry left, changed or was hidden: every verdict so far may hide what it uncovered.
    pub fn invalidate_world(&mut self) {
        self.world += 1;
        self.basis = None;
        self.settled = 0;
        self.runs.fill(0);
    }

    /// `slot` received a new record on `frame`.
    pub fn assign(&mut self, slot: u32, frame: u64) {
        let slot = slot as usize;
        if self.runs.len() <= slot {
            self.runs.resize(slot + 1, 0);
            self.since.resize(slot + 1, 0);
        }
        self.runs[slot] = 0;
        self.since[slot] = frame;
        self.assigned_at = frame;
        self.settled = 0;
    }

    /// Folds one readback in; a new basis restarts every run.
    pub fn apply(&mut self, tag: &VerdictTag, occluded: &[u32]) {
        if self.basis != Some(tag.view.basis) {
            self.basis = Some(tag.view.basis);
            self.runs.fill(0);
            self.settled = 0;
        }
        if self.forward != tag.view.forward {
            self.forward = tag.view.forward;
            self.settled = 0;
        }
        if tag.frame >= self.assigned_at {
            self.settled = self.settled.saturating_add(1);
        }
        for (slot, run) in self.runs.iter_mut().enumerate() {
            let hit = slot < tag.slots as usize
                && occluded
                    .get(slot / 32)
                    .is_some_and(|word| word >> (slot % 32) & 1 != 0)
                && self.since[slot] <= tag.frame;
            *run = if hit { run.saturating_add(1) } else { 0 };
        }
    }

    /// Whether `slot` may be skipped when drawing `view`.
    pub fn skips(&self, slot: u32, view: &OcclusionView) -> bool {
        self.basis == Some(view.basis)
            && view.basis.world == self.world
            && dot(self.forward, view.forward) >= MAX_VERDICT_TURN.cos()
            && self
                .runs
                .get(slot as usize)
                .is_some_and(|&run| run >= REQUIRED_OCCLUDED_READBACKS)
    }

    /// Whether another verdict for `view` would repeat the last ones: same eye, projection,
    /// geometry and direction, and enough verdicts since any record changed.
    pub fn settled(&self, view: &OcclusionView) -> bool {
        self.basis == Some(view.basis)
            && view.basis.world == self.world
            && self.forward == view.forward
            && self.settled >= REQUIRED_OCCLUDED_READBACKS
    }
}

fn dot(left: [f32; 3], right: [f32; 3]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}
