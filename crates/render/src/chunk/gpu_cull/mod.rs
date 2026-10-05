//! GPU-driven opaque terrain culling: persistent per-slot records, a compute cull with
//! two-phase Hi-Z occlusion, and count-driven multi-draw-indirect submission.
//!
//! Backends whose multi-draw is a CPU loop (Metal, GL) lack `MULTI_DRAW_INDIRECT_COUNT` and
//! keep CPU culling; so do frames with an active presentation or visibility probe.

pub(in crate::chunk) mod kernels;
pub(in crate::chunk) mod model;
mod node;
#[cfg(test)]
mod tests;

use bevy::{
    camera::{MainPassResolutionOverride, primitives::Frustum, visibility::RenderLayers},
    render::{
        Extract, ExtractSchedule,
        render_phase::DrawFunctionId,
        render_resource::{CachedRenderPipelineId, TextureViewId},
        sync_world::RenderEntity,
        view::ViewDepthTexture,
    },
};

use crate::chunk::*;
use kernels::{CullKernels, CullStorage, HizPyramid, PyramidBindings};
use model::{
    CullCamera, CullPhase, CullRecord, CullRecordSource, CullStream, CullViewInput,
    CullViewUniform, STREAM_COUNT,
};
pub(in crate::chunk) use node::{draw_function_ids, install_commands};

/// Forces the CPU culling path for A/B measurement.
const CPU_CULLING_ENV: &str = "RUST_MCBE_CPU_CULLING";
const MIN_CAPACITY: u32 = 1024;
const SIDE: i32 = world::SUB_CHUNK_SIDE as i32;
/// Models may overhang their sub-chunk by one block.
const MODEL_BOUNDS: [[i32; 3]; 2] = [[-1; 3], [SIDE + 1; 3]];
const FULL_BOUNDS: [[i32; 3]; 2] = [[0; 3], [SIDE; 3]];

/// Whether opaque terrain is culled on the GPU on this device.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::chunk) struct GpuCullSupport(pub(in crate::chunk) bool);

pub(in crate::chunk) fn gpu_cull_supported(
    draw_mode: ChunkDrawMode,
    features: WgpuFeatures,
    downlevel: DownlevelFlags,
    forced_cpu: bool,
) -> bool {
    !forced_cpu
        && draw_mode == ChunkDrawMode::MultiDrawIndirect
        && features.contains(WgpuFeatures::MULTI_DRAW_INDIRECT_COUNT)
        && downlevel.contains(DownlevelFlags::COMPUTE_SHADERS)
}

/// The view queued for GPU culling this frame, with the pipelines its late pass reuses.
#[derive(Clone, Copy, Debug)]
pub(in crate::chunk) struct GpuCullView {
    pub(in crate::chunk) entity: Entity,
    pub(in crate::chunk) main: MainEntity,
    pub(in crate::chunk) pipelines: [CachedRenderPipelineId; STREAM_COUNT],
    pub(in crate::chunk) late_draws: [DrawFunctionId; STREAM_COUNT],
}

#[derive(Resource, Default)]
pub(in crate::chunk) struct GpuCullFrame {
    pub(in crate::chunk) view: Option<GpuCullView>,
}

/// Queue-time access: chooses the GPU-culled view and records its pipelines.
#[derive(SystemParam)]
pub(in crate::chunk) struct GpuCullQueue<'w, 's> {
    support: Option<Res<'w, GpuCullSupport>>,
    frame: Option<ResMut<'w, GpuCullFrame>>,
    views: Query<'w, 's, (Option<&'static Frustum>, Option<&'static RenderLayers>)>,
}

impl GpuCullQueue<'_, '_> {
    /// The view to cull on the GPU this frame; probes keep every view on the CPU path.
    pub(in crate::chunk) fn select<'a>(
        &mut self,
        draw_mode: ChunkDrawMode,
        probing: bool,
        candidates: impl IntoIterator<Item = (Entity, &'a MainEntity, &'a ExtractedView, bool)>,
    ) -> Option<Entity> {
        if let Some(frame) = self.frame.as_deref_mut() {
            frame.view = None;
        }
        let supported = self.support.as_deref().is_some_and(|support| support.0);
        if probing || !supported || draw_mode != ChunkDrawMode::MultiDrawIndirect {
            return None;
        }
        select_gpu_cull_view(candidates, |entity| {
            self.views
                .get(entity)
                .is_ok_and(|(frustum, layers)| gpu_cull_view_eligible(frustum, layers))
        })
    }

    pub(in crate::chunk) fn set_view(&mut self, view: GpuCullView) {
        if let Some(frame) = self.frame.as_deref_mut() {
            frame.view = Some(view);
        }
    }
}

/// Render entities whose main-world chunk is not inherited-visible (the cave culler hides them).
#[derive(Resource, Default)]
pub(in crate::chunk) struct ChunkHiddenEntities {
    hidden: HashSet<Entity>,
    changed: Vec<Entity>,
}

#[derive(Clone, Copy)]
struct SlotOwner {
    entity: Entity,
    tint: ChunkBiomeTintIdentity,
}

struct PreparedPyramid {
    pyramid: HizPyramid,
    depth: TextureViewId,
    bindings: PyramidBindings,
}

/// Slot-indexed record mirror and enabled bits, tracking what the GPU copy still needs.
#[derive(Default)]
pub(in crate::chunk) struct CullSlots {
    records: Vec<CullRecord>,
    owners: Vec<Option<SlotOwner>>,
    slots: HashMap<Entity, u32>,
    enabled: Vec<u32>,
    enabled_dirty: bool,
    dirty: Vec<u32>,
    tint_identity: Option<ChunkBiomeTintIdentity>,
}

impl CullSlots {
    pub(in crate::chunk) fn slot_count(&self) -> u32 {
        self.records.len() as u32
    }

    pub(in crate::chunk) fn records(&self) -> &[CullRecord] {
        &self.records
    }

    pub(in crate::chunk) fn enabled(&self) -> &[u32] {
        &self.enabled
    }

    /// Frees `entity`'s slot after its allocation component is gone.
    pub(in crate::chunk) fn remove(&mut self, entity: Entity) {
        if let Some(slot) = self.slots.remove(&entity) {
            self.clear_slot(slot, entity);
        }
    }

    /// Writes `entity`'s record at its (possibly new) metadata slot.
    pub(in crate::chunk) fn update(
        &mut self,
        entity: Entity,
        slot: u32,
        tint: ChunkBiomeTintIdentity,
        record: CullRecord,
        hidden: &HashSet<Entity>,
    ) {
        if let Some(previous) = self.slots.insert(entity, slot)
            && previous != slot
        {
            self.clear_slot(previous, entity);
        }
        let index = slot as usize;
        if self.records.len() <= index {
            // Fresh slots may still hold records a trim left on the GPU.
            self.dirty.extend(self.records.len() as u32..slot);
            self.records.resize(index + 1, CullRecord::default());
            self.owners.resize(index + 1, None);
        }
        self.records[index] = record;
        self.owners[index] = Some(SlotOwner { entity, tint });
        self.dirty.push(slot);
        self.refresh(slot, hidden);
    }

    /// Re-derives `entity`'s enabled bit after its cave visibility flipped.
    pub(in crate::chunk) fn refresh_entity(&mut self, entity: Entity, hidden: &HashSet<Entity>) {
        if let Some(&slot) = self.slots.get(&entity) {
            self.refresh(slot, hidden);
        }
    }

    /// Disables every slot whose mesh predates the active biome-tint table.
    pub(in crate::chunk) fn set_tint(
        &mut self,
        tint: ChunkBiomeTintIdentity,
        hidden: &HashSet<Entity>,
    ) {
        if self.tint_identity == Some(tint) {
            return;
        }
        self.tint_identity = Some(tint);
        for slot in 0..self.slot_count() {
            self.refresh(slot, hidden);
        }
    }

    /// Drops trailing free slots so the cull dispatch covers only the live watermark.
    pub(in crate::chunk) fn trim(&mut self) {
        while self.owners.last().is_some_and(Option::is_none) {
            self.owners.pop();
            self.records.pop();
        }
        let len = self.records.len();
        self.dirty.retain(|&slot| (slot as usize) < len);
    }

    /// Sorted, deduplicated dirty slots; the caller uploads them.
    pub(in crate::chunk) fn take_dirty(&mut self) -> Vec<u32> {
        let mut dirty = std::mem::take(&mut self.dirty);
        dirty.sort_unstable();
        dirty.dedup();
        dirty
    }

    pub(in crate::chunk) fn take_enabled_dirty(&mut self) -> bool {
        std::mem::take(&mut self.enabled_dirty)
    }

    fn mark_all_dirty(&mut self) {
        self.dirty = (0..self.slot_count()).collect();
        self.enabled_dirty = true;
    }

    fn refresh(&mut self, slot: u32, hidden: &HashSet<Entity>) {
        let tint = self.tint_identity;
        let enabled = self.owners[slot as usize].is_some_and(|owner| {
            !hidden.contains(&owner.entity)
                && tint.is_some_and(|tint| chunk_tint_identity_is_active(owner.tint, tint))
        });
        self.set_enabled(slot, enabled);
    }

    fn set_enabled(&mut self, slot: u32, value: bool) {
        let (word, bit) = (slot as usize / 32, 1 << (slot % 32));
        if self.enabled.len() <= word {
            self.enabled.resize(word + 1, 0);
        }
        let old = self.enabled[word];
        self.enabled[word] = if value { old | bit } else { old & !bit };
        self.enabled_dirty |= old != self.enabled[word];
    }

    fn clear_slot(&mut self, slot: u32, entity: Entity) {
        if self.owners[slot as usize].is_some_and(|owner| owner.entity == entity) {
            self.owners[slot as usize] = None;
            self.records[slot as usize] = CullRecord::default();
            self.dirty.push(slot);
            self.set_enabled(slot, false);
        }
    }
}

/// The record table plus the GPU state of the culled view.
#[derive(Resource)]
pub(in crate::chunk) struct GpuCull {
    kernels: CullKernels,
    storage: Option<CullStorage>,
    args: Option<Buffer>,
    draw_counts: Option<Buffer>,
    table: CullSlots,
    pyramid: Option<PreparedPyramid>,
    bind_groups: Option<[wgpu::BindGroup; 2]>,
    bound_pyramid: Option<TextureViewId>,
    prepared_view: Option<Entity>,
}

impl GpuCull {
    fn new(device: &RenderDevice) -> Self {
        Self {
            kernels: CullKernels::new(device.wgpu_device()),
            storage: None,
            args: None,
            draw_counts: None,
            table: CullSlots::default(),
            pyramid: None,
            bind_groups: None,
            bound_pyramid: None,
            prepared_view: None,
        }
    }

    fn slot_count(&self) -> u32 {
        self.table.slot_count()
    }

    /// Args, counts and slot capacity when `view` was prepared this frame.
    pub(in crate::chunk) fn prepared_draws(&self, view: Entity) -> Option<(&Buffer, &Buffer, u32)> {
        (self.prepared_view == Some(view)).then_some(())?;
        Some((
            self.args.as_ref()?,
            self.draw_counts.as_ref()?,
            self.storage.as_ref()?.capacity,
        ))
    }
}

/// Builds a slot's record from the same validated ranges the CPU draw path uses.
pub(in crate::chunk) fn cull_record(
    allocation: &GpuChunkAllocation,
    instance: Option<&ChunkRenderInstance>,
) -> CullRecord {
    let Some(base_vertex) = metadata_base_vertex(allocation.metadata_index) else {
        return CullRecord::default();
    };
    let mut source = CullRecordSource {
        origin: chunk_origin(allocation.key),
        base_vertex,
        ..default()
    };
    let mut bounds: Option<[[i32; 3]; 2]> = None;
    let mut include = |extent: [[i32; 3]; 2]| {
        bounds = Some(bounds.map_or(extent, |[low, high]| {
            [
                std::array::from_fn(|axis| low[axis].min(extent[0][axis])),
                std::array::from_fn(|axis| high[axis].max(extent[1][axis])),
            ]
        }));
    };
    if let Some((cube, layout, _)) = cube_draw_base(allocation) {
        let quads = instance
            .map(|instance| &*instance.cube_quads)
            .filter(|quads| quads.len() as u32 == cube.end - cube.start);
        match quads {
            Some(quads) => quads.iter().for_each(|quad| include(quad_bounds(quad))),
            None => include(FULL_BOUNDS),
        }
        source.solid_ends =
            CubeQuadLayout::SOLID_FACE_ORDER.map(|face| layout.solid_range(face).end);
        source.cube = cube;
    }
    if let Some(draw) = model_mdi_draw_command(allocation) {
        source.model = draw.first_instance..draw.first_instance + draw.instance_count;
        include(MODEL_BOUNDS);
    }
    if let Some(draw) = depth_liquid_mdi_draw_command(allocation) {
        source.liquid = draw.first_instance..draw.first_instance + draw.instance_count;
        include(FULL_BOUNDS);
    }
    source.bounds = bounds.unwrap_or(FULL_BOUNDS);
    CullRecord::new(&source).unwrap_or_default()
}

/// A box containing the quad: it starts at its origin and spans its larger extent on each axis.
fn quad_bounds(quad: &PackedQuad) -> [[i32; 3]; 2] {
    let origin = quad.origin().map(i32::from);
    let extent = i32::from(quad.width().max(quad.height()));
    [origin, origin.map(|value| (value + extent).min(SIDE))]
}

pub(in crate::chunk) fn install(app: &mut App) {
    let forced_cpu = std::env::var_os(CPU_CULLING_ENV).is_some_and(|value| value != "0");
    let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
        return;
    };
    let (Some(device), Some(adapter)) = (
        render_app.world().get_resource::<RenderDevice>().cloned(),
        render_app.world().get_resource::<RenderAdapter>(),
    ) else {
        return;
    };
    let draw_mode = select_chunk_draw_mode(
        adapter.get_downlevel_capabilities().flags,
        device.features(),
        Backends::from(adapter.get_info().backend),
        cfg!(debug_assertions),
    );
    let support = GpuCullSupport(gpu_cull_supported(
        draw_mode,
        device.features(),
        adapter.get_downlevel_capabilities().flags,
        forced_cpu,
    ));
    render_app
        .insert_resource(support)
        .init_resource::<GpuCullFrame>()
        .add_systems(Render, reset_gpu_cull_frame.in_set(RenderSystems::Cleanup));
    if !support.0 {
        return;
    }
    render_app
        .insert_resource(GpuCull::new(&device))
        .init_resource::<ChunkHiddenEntities>()
        .add_systems(ExtractSchedule, extract_hidden_chunks)
        .add_systems(
            Render,
            prepare_gpu_cull
                .in_set(RenderSystems::PrepareResources)
                .after(prepare_gpu_chunks)
                .after(bevy::core_pipeline::core_3d::prepare_core_3d_depth_textures),
        );
    node::install_graph(render_app.world_mut());
    app.insert_resource(support)
        .add_systems(Last, admit_depth_sampling);
}

/// The late pass seeds its pyramid from the main depth target.
fn admit_depth_sampling(mut cameras: Query<&mut Camera3d>) {
    for mut camera in &mut cameras {
        let usage = TextureUsages::from(camera.depth_texture_usages);
        if !usage.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

fn reset_gpu_cull_frame(mut frame: ResMut<GpuCullFrame>, cull: Option<ResMut<GpuCull>>) {
    frame.view = None;
    if let Some(mut cull) = cull {
        cull.prepared_view = None;
    }
}

fn extract_hidden_chunks(
    mut hidden: ResMut<ChunkHiddenEntities>,
    chunks: Extract<
        Query<
            (RenderEntity, &InheritedVisibility),
            (With<ChunkRenderInstance>, Changed<InheritedVisibility>),
        >,
    >,
) {
    for (entity, visibility) in &chunks {
        let changed = if visibility.get() {
            hidden.hidden.remove(&entity)
        } else {
            hidden.hidden.insert(entity)
        };
        if changed {
            hidden.changed.push(entity);
        }
    }
}

/// Picks the GPU-culled view: the lowest-id unmirrored perspective view on render layer 0.
pub(in crate::chunk) fn select_gpu_cull_view<'a>(
    candidates: impl IntoIterator<Item = (Entity, &'a MainEntity, &'a ExtractedView, bool)>,
    eligible: impl Fn(Entity) -> bool,
) -> Option<Entity> {
    candidates
        .into_iter()
        .filter(|&(entity, _, view, enhanced)| {
            pipeline::solid::solid_cull_camera(view, enhanced).is_some() && eligible(entity)
        })
        .min_by_key(|(_, main, _, _)| main.id().to_bits())
        .map(|(entity, ..)| entity)
}

/// Whether the view can host the GPU cull: it has a frustum and sees render layer 0.
pub(in crate::chunk) fn gpu_cull_view_eligible(
    frustum: Option<&Frustum>,
    layers: Option<&RenderLayers>,
) -> bool {
    frustum.is_some() && layers.is_none_or(|layers| layers.intersects(&RenderLayers::default()))
}

type CullViewComponents = (
    &'static ExtractedView,
    &'static Frustum,
    Option<&'static ViewDepthTexture>,
    &'static Msaa,
    Option<&'static MainPassResolutionOverride>,
);

#[allow(clippy::too_many_arguments)]
fn prepare_gpu_cull(
    mut cull: ResMut<GpuCull>,
    mut hidden: ResMut<ChunkHiddenEntities>,
    frame: Res<GpuCullFrame>,
    changed: Query<
        (Entity, &GpuChunkAllocation, Option<&ChunkRenderInstance>),
        Changed<GpuChunkAllocation>,
    >,
    mut removed: RemovedComponents<GpuChunkAllocation>,
    biome_tints: Res<ChunkBiomeTints>,
    views: Query<CullViewComponents>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    profiler: Option<Res<RuntimeStageProfiler>>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(RuntimeStage::IndirectPreparation));
    let cull = &mut *cull;
    let hidden = &mut *hidden;
    for entity in removed.read() {
        cull.table.remove(entity);
    }
    cull.table
        .set_tint(biome_tints.table_identity(), &hidden.hidden);
    for (entity, allocation, instance) in &changed {
        let record = cull_record(allocation, instance);
        let slot = allocation.metadata_index;
        cull.table.update(
            entity,
            slot,
            allocation.tint_identity,
            record,
            &hidden.hidden,
        );
    }
    for entity in hidden.changed.drain(..) {
        cull.table.refresh_entity(entity, &hidden.hidden);
    }
    cull.table.trim();
    upload_records(cull, &device, &queue);

    let Some(view) = frame.view else {
        return;
    };
    let Ok((extracted, frustum, depth, msaa, resolution_override)) = views.get(view.entity) else {
        return;
    };
    let depth = depth.filter(|depth| {
        resolution_override.is_none()
            && depth
                .texture
                .usage()
                .contains(TextureUsages::TEXTURE_BINDING)
    });
    prepare_pyramid(cull, &device, depth, *msaa);
    let storage = cull.storage.as_ref().expect("records were uploaded");
    let hiz_mips = cull
        .pyramid
        .as_ref()
        .map_or(0, |prepared| prepared.pyramid.mip_count());
    let depth_size = cull
        .pyramid
        .as_ref()
        .map_or([1, 1], |prepared| prepared.pyramid.depth_size);
    let input = cull_view_input(extracted, frustum, depth_size, hiz_mips);
    for phase in CullPhase::ALL {
        let uniform = CullViewUniform::new(&input, phase, cull.slot_count(), storage.capacity);
        storage.write_uniform(&queue, phase, &uniform);
    }
    let pyramid_id = cull.pyramid.as_ref().map(|prepared| prepared.depth);
    if cull.bind_groups.is_none() || cull.bound_pyramid != pyramid_id {
        cull.bind_groups = Some(cull.kernels.bind_groups(
            device.wgpu_device(),
            storage,
            cull.pyramid.as_ref().map(|prepared| &prepared.pyramid),
        ));
        cull.bound_pyramid = pyramid_id;
    }
    cull.prepared_view = Some(view.entity);
}

/// Grows storage when the slot watermark passes it, then writes dirty records and bits.
fn upload_records(cull: &mut GpuCull, device: &RenderDevice, queue: &RenderQueue) {
    let slots = cull.slot_count();
    if cull
        .storage
        .as_ref()
        .is_none_or(|storage| storage.capacity < slots)
    {
        let capacity = slots.max(MIN_CAPACITY).next_power_of_two();
        let storage = CullStorage::new(device.wgpu_device(), capacity, false);
        cull.args = Some(Buffer::from(storage.args.clone()));
        cull.draw_counts = Some(Buffer::from(storage.draw_counts.clone()));
        cull.storage = Some(storage);
        cull.bind_groups = None;
        cull.table.mark_all_dirty();
    }
    let storage = cull.storage.as_ref().expect("storage was just ensured");
    let record_bytes = std::mem::size_of::<CullRecord>() as u64;
    let dirty = cull.table.take_dirty();
    let records = cull.table.records();
    for run in dirty.chunk_by(|left, right| left + 1 == *right) {
        let (first, last) = (run[0] as usize, run[run.len() - 1] as usize);
        queue.write_buffer(
            &storage.records,
            first as u64 * record_bytes,
            bytemuck::cast_slice(&records[first..=last]),
        );
    }
    if cull.table.take_enabled_dirty() {
        let enabled = cull.table.enabled();
        let words = enabled.len().min((storage.capacity as usize).div_ceil(32));
        if words != 0 {
            queue.write_buffer(&storage.enabled, 0, bytemuck::cast_slice(&enabled[..words]));
        }
    }
}

fn prepare_pyramid(
    cull: &mut GpuCull,
    device: &RenderDevice,
    depth: Option<&ViewDepthTexture>,
    msaa: Msaa,
) {
    let Some(depth) = depth else {
        cull.pyramid = None;
        return;
    };
    let size = depth.texture.size();
    let depth_size = [size.width, size.height];
    let view = depth.view();
    if cull.pyramid.as_ref().is_some_and(|prepared| {
        prepared.depth == view.id() && prepared.pyramid.depth_size == depth_size
    }) {
        return;
    }
    let pyramid = cull
        .pyramid
        .take()
        .map(|prepared| prepared.pyramid)
        .filter(|pyramid| pyramid.depth_size == depth_size)
        .unwrap_or_else(|| HizPyramid::new(device.wgpu_device(), depth_size));
    let bindings =
        cull.kernels
            .pyramid_bindings(device.wgpu_device(), view, msaa.samples() > 1, &pyramid);
    cull.pyramid = Some(PreparedPyramid {
        pyramid,
        depth: view.id(),
        bindings,
    });
}

fn cull_view_input(
    view: &ExtractedView,
    frustum: &Frustum,
    depth_size: [u32; 2],
    hiz_mips: u32,
) -> CullViewInput {
    let world_from_view = view.world_from_view.to_matrix().as_dmat4();
    let clip_from_world = view
        .clip_from_world
        .map(|matrix| matrix.as_dmat4())
        .unwrap_or_else(|| view.clip_from_view.as_dmat4() * world_from_view.inverse());
    let eye = pipeline::solid::solid_cull_camera(view, false);
    CullViewInput {
        planes: std::array::from_fn(|index| frustum.half_spaces[index].normal_d().to_array()),
        clip_from_world: clip_from_world.to_cols_array_2d(),
        camera: CullCamera::new(eye),
        viewport: view.viewport.as_vec4().to_array(),
        depth_size,
        hiz_mips,
        index_counts: CullStream::ALL.map(|stream| match stream {
            CullStream::Model => MODEL_INDEX_COUNT,
            _ => STATIC_QUAD_INDICES.len() as u32,
        }),
    }
}
