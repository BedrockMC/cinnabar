use crate::chunk::*;

/// Hard 16 MiB ceiling for one committed transparent indirection snapshot.
pub const MAX_TRANSPARENT_DRAW_REFS: usize = 2_097_152;
pub const MAX_TRANSPARENT_VIEWS: usize = 1;
/// The largest one slot of the double-buffered ref buffer grows to.
pub const TRANSPARENT_REF_SLOT_BYTES: usize =
    MAX_TRANSPARENT_DRAW_REFS * std::mem::size_of::<PackedTransparentDrawRef>();
pub const TRANSPARENT_REF_BUFFER_BYTES: usize = TRANSPARENT_REF_SLOT_BYTES * 2;
/// Refs per slot before the first growth; slots double up to the ceiling as snapshots need.
pub(in crate::chunk) const INITIAL_TRANSPARENT_SLOT_REFS: usize = 16_384;
pub const DEFAULT_TRANSPARENT_UPLOAD_REFS_PER_FRAME: usize = 131_072;
pub const MAX_TRANSPARENT_WITNESS_KEYS: usize = 64;
pub const MAX_MODEL_WITNESS_KEYS: usize = 64;
pub(in crate::chunk) const MAX_TRANSPARENT_RETIRED_ALLOCATIONS: usize = 16_384;
pub(in crate::chunk) const MAX_TRANSPARENT_RETIRED_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransparentDrawArgs {
    pub index_count: u32,
    pub instance_count: u32,
    pub first_index: u32,
    pub base_vertex: i32,
    pub first_instance: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransparentSortCandidate {
    pub(in crate::chunk) key: SubChunkKey,
    pub(in crate::chunk) local_quad_index: u32,
    pub(in crate::chunk) liquid_record_index: u32,
    pub(in crate::chunk) metadata_index: u32,
    pub(in crate::chunk) subchunk_center: [f32; 3],
    pub(in crate::chunk) quad_centroid: [f32; 3],
}

impl TransparentSortCandidate {
    #[must_use]
    pub const fn new(
        key: SubChunkKey,
        local_quad_index: u32,
        liquid_record_index: u32,
        metadata_index: u32,
        subchunk_center: [f32; 3],
        quad_centroid: [f32; 3],
    ) -> Self {
        Self {
            key,
            local_quad_index,
            liquid_record_index,
            metadata_index,
            subchunk_center,
            quad_centroid,
        }
    }
}

pub(in crate::chunk) fn sort_transparent_candidates(
    view_from_world: Mat4,
    candidates: Arc<[TransparentSortCandidate]>,
) -> Vec<PackedTransparentDrawRef> {
    let metric = super::face_metric::TransparentFaceMetric::new(
        view_from_world.inverse().transform_point3(Vec3::ZERO),
    );
    let quad_depths = candidates
        .iter()
        .map(|candidate| metric.distance(candidate.key, Vec3::from_array(candidate.quad_centroid)))
        .collect::<Vec<_>>();
    let mut grouped = BTreeMap::<SubChunkKey, Vec<usize>>::new();
    for (index, candidate) in candidates.iter().enumerate() {
        grouped.entry(candidate.key).or_default().push(index);
    }
    let mut groups = grouped
        .into_iter()
        .map(|(key, indices)| {
            let center = Vec3::from_array(candidates[indices[0]].subchunk_center);
            let depth = view_from_world.transform_point3(center).z;
            (depth, key, indices)
        })
        .collect::<Vec<_>>();
    groups.sort_by(|(left_depth, left_key, _), (right_depth, right_key, _)| {
        left_depth
            .total_cmp(right_depth)
            .then_with(|| left_key.cmp(right_key))
    });
    let mut refs = Vec::with_capacity(candidates.len());
    for (_depth, _key, mut group) in groups {
        group.sort_by(|&left, &right| {
            let left_candidate = &candidates[left];
            let right_candidate = &candidates[right];
            quad_depths[right]
                .total_cmp(&quad_depths[left])
                .then_with(|| left_candidate.key.cmp(&right_candidate.key))
                .then_with(|| {
                    left_candidate
                        .local_quad_index
                        .cmp(&right_candidate.local_quad_index)
                })
        });
        refs.extend(group.into_iter().map(|index| {
            let candidate = &candidates[index];
            PackedTransparentDrawRef::new(candidate.liquid_record_index, candidate.metadata_index)
        }));
    }
    refs
}

pub(in crate::chunk) fn transparent_draw_args(
    buffer_slot: u8,
    slot_refs: usize,
    ref_count: usize,
) -> Option<TransparentDrawArgs> {
    transparent_draw_range_args(buffer_slot, slot_refs, 0..u32::try_from(ref_count).ok()?)
}

/// `slot_refs` is the arena's current per-slot capacity, the stride between the two slots.
pub(in crate::chunk) fn transparent_draw_range_args(
    buffer_slot: u8,
    slot_refs: usize,
    ref_range: Range<u32>,
) -> Option<TransparentDrawArgs> {
    if ref_range.start > ref_range.end
        || slot_refs > MAX_TRANSPARENT_DRAW_REFS
        || usize::try_from(ref_range.end).ok()? > slot_refs
    {
        return None;
    }
    let instance_count = ref_range.end - ref_range.start;
    let first_instance = u32::from(buffer_slot)
        .checked_mul(u32::try_from(slot_refs).ok()?)?
        .checked_add(ref_range.start)?;
    Some(TransparentDrawArgs {
        index_count: STATIC_QUAD_INDICES.len() as u32,
        instance_count,
        first_index: 0,
        base_vertex: 0,
        first_instance,
    })
}

/// Byte offset of `start` within `buffer_slot` at the arena's current slot stride.
pub(in crate::chunk) fn transparent_ref_offset(
    buffer_slot: u8,
    slot_refs: usize,
    start: usize,
) -> u64 {
    ((usize::from(buffer_slot) * slot_refs + start)
        * std::mem::size_of::<PackedTransparentDrawRef>()) as u64
}

pub(in crate::chunk) fn transparent_ref_buffer(device: &RenderDevice, slot_refs: usize) -> Buffer {
    create_storage_buffer(
        device,
        "double-buffered transparent draw refs",
        transparent_ref_offset(2, slot_refs, 0),
    )
}

/// Grows both slots to hold `refs`, re-uploading every resident ref at the new stride.
/// Returns whether the buffer was replaced, which invalidates written indirect args.
pub(in crate::chunk) fn ensure_transparent_ref_capacity(
    arena: &mut ChunkGpuArena,
    device: &RenderDevice,
    queue: &RenderQueue,
    refs: usize,
    state: &TransparentSortState,
) -> bool {
    if refs <= arena.transparent_slot_refs {
        return false;
    }
    let slot_refs = refs
        .min(MAX_TRANSPARENT_DRAW_REFS)
        .next_power_of_two()
        .clamp(INITIAL_TRANSPARENT_SLOT_REFS, MAX_TRANSPARENT_DRAW_REFS);
    arena.transparent_ref_buffer = transparent_ref_buffer(device, slot_refs);
    arena.transparent_slot_refs = slot_refs;
    for (slot, resident) in state.resident_refs() {
        queue.write_buffer(
            &arena.transparent_ref_buffer,
            transparent_ref_offset(slot, slot_refs, 0),
            bytemuck::cast_slice(resident),
        );
    }
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chunk) struct TransparentLiquidPhaseGroup {
    pub(in crate::chunk) key: SubChunkKey,
    pub(in crate::chunk) ref_range: Range<u32>,
}

pub(in crate::chunk) fn transparent_liquid_phase_groups(
    snapshot: &TransparentOrderedSnapshot,
) -> Option<Vec<TransparentLiquidPhaseGroup>> {
    let mut identities = BTreeMap::new();
    for identity in snapshot.key.visible_allocations.iter() {
        if !identity.liquid_range.start.is_multiple_of(4)
            || !identity.liquid_range.end.is_multiple_of(4)
            || identities
                .insert(identity.metadata_index, identity)
                .is_some()
        {
            return None;
        }
    }

    let mut groups = Vec::<TransparentLiquidPhaseGroup>::new();
    let mut closed_metadata = HashSet::new();
    for (index, draw_ref) in snapshot.refs().iter().copied().enumerate() {
        let identity = identities.get(&draw_ref.metadata_index())?;
        let record_range = identity.liquid_range.start / 4..identity.liquid_range.end / 4;
        if !record_range.contains(&draw_ref.liquid_record_index()) {
            return None;
        }
        let index = u32::try_from(index).ok()?;
        if let Some(group) = groups.last_mut()
            && group.key == identity.key
        {
            group.ref_range.end = index.checked_add(1)?;
            continue;
        }
        if !closed_metadata.insert(identity.metadata_index) {
            return None;
        }
        groups.push(TransparentLiquidPhaseGroup {
            key: identity.key,
            ref_range: index..index.checked_add(1)?,
        });
    }
    Some(groups)
}

pub(in crate::chunk) fn transparent_indirect_args(
    snapshot: &TransparentOrderedSnapshot,
    slot_refs: usize,
) -> Option<DrawIndexedIndirectArgs> {
    let args = transparent_draw_args(snapshot.buffer_slot(), slot_refs, snapshot.refs().len())?;
    Some(DrawIndexedIndirectArgs {
        index_count: args.index_count,
        instance_count: args.instance_count,
        first_index: args.first_index,
        base_vertex: args.base_vertex,
        first_instance: args.first_instance,
    })
}

/// One absolute liquid-record/chunk-metadata pair in committed back-to-front order.
///
/// The liquid record carries its absolute lighting address in word 3. These
/// references belong to a committed per-view snapshot, never to `ChunkMesh`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PackedTransparentDrawRef {
    pub(in crate::chunk) liquid_record_index: u32,
    pub(in crate::chunk) metadata_index: u32,
}

impl PackedTransparentDrawRef {
    #[must_use]
    pub const fn new(liquid_record_index: u32, metadata_index: u32) -> Self {
        Self {
            liquid_record_index,
            metadata_index,
        }
    }

    #[must_use]
    pub const fn liquid_record_index(self) -> u32 {
        self.liquid_record_index
    }

    #[must_use]
    pub const fn metadata_index(self) -> u32 {
        self.metadata_index
    }
}

pub(in crate::chunk) const _: () = assert!(std::mem::size_of::<PackedTransparentDrawRef>() == 8);

mod prepare;
mod state;

pub(in crate::chunk) use prepare::{
    build_transparent_candidates, prepare_transparent_sorts, quantized_camera_orientation,
    quantized_camera_position, transparent_snapshot_addresses_are_resident,
};
pub(in crate::chunk) use state::{
    TransparentAddressIdentity, TransparentCandidateCache, TransparentSortRequest,
    TransparentSortRuntime, TransparentSortWork, TransparentStagedSnapshot,
    TransparentWorkerResult,
};
pub use state::{
    TransparentAllocationIdentity, TransparentOrderedSnapshot, TransparentSortError,
    TransparentSortJobGate, TransparentSortResult, TransparentSortState, TransparentUploadBatch,
    ViewSortGeneration, ViewSortKey, validate_transparent_sort_ref_count,
};

#[cfg(test)]
mod growth_tests {
    use super::*;

    /// The ref buffer starts small and grows only when a staged snapshot needs it.
    #[test]
    fn transparent_ref_buffer_grows_on_demand_and_keeps_resident_refs() {
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue(Arc::new(bevy::render::renderer::WgpuWrapper::new(queue)));
        let mut arena = ChunkGpuArena::new(&device);
        let initial = transparent_ref_offset(2, INITIAL_TRANSPARENT_SLOT_REFS, 0);
        assert_eq!(arena.transparent_ref_buffer.size(), initial);
        assert!(initial < TRANSPARENT_REF_BUFFER_BYTES as u64 / 64);

        let mut state = TransparentSortState::with_upload_cap(usize::MAX);
        let key = ViewSortKey::try_new(
            [0.0; 3],
            [0.0, 0.0, 0.0, 1.0],
            Vec::new(),
            ChunkTextureAssetIdentity::new(1, 1),
            ChunkBiomeTintIdentity::new(2, 2),
        )
        .unwrap();
        let generation = state.request(&key);
        let refs = vec![PackedTransparentDrawRef::new(1, 2); 3];
        state
            .complete(TransparentSortResult::new(generation, key, refs).unwrap())
            .unwrap();
        assert!(state.acknowledge_upload());
        assert_eq!(state.resident_refs().count(), 1);

        assert!(!ensure_transparent_ref_capacity(
            &mut arena,
            &device,
            &queue,
            INITIAL_TRANSPARENT_SLOT_REFS,
            &state,
        ));
        let before = arena.transparent_ref_buffer.id();
        assert!(ensure_transparent_ref_capacity(
            &mut arena,
            &device,
            &queue,
            INITIAL_TRANSPARENT_SLOT_REFS + 1,
            &state,
        ));
        assert_ne!(arena.transparent_ref_buffer.id(), before);
        assert_eq!(
            arena.transparent_slot_refs,
            INITIAL_TRANSPARENT_SLOT_REFS * 2
        );
        assert_eq!(arena.transparent_ref_buffer.size(), initial * 2);
        let args = transparent_draw_args(1, arena.transparent_slot_refs, 3).unwrap();
        assert_eq!(
            args.first_instance,
            INITIAL_TRANSPARENT_SLOT_REFS as u32 * 2
        );

        ensure_transparent_ref_capacity(&mut arena, &device, &queue, usize::MAX, &state);
        assert_eq!(arena.transparent_slot_refs, MAX_TRANSPARENT_DRAW_REFS);
        assert_eq!(
            arena.transparent_ref_buffer.size(),
            TRANSPARENT_REF_BUFFER_BYTES as u64
        );
    }
}
