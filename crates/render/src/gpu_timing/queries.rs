//! Bounded timestamp query storage and asynchronous frame readback.

use super::{
    health::QueryHealth,
    readback::{GpuFrameTimes, ReadbackRing, SLOTS, decode_spans},
};
use crate::RuntimeStage;
use bevy::{
    prelude::Resource,
    render::renderer::{RenderDevice, RenderQueue},
};
use std::sync::{
    Arc,
    atomic::{AtomicU8, AtomicU32, Ordering},
};

/// Node-level spans per frame, ahead of the draw pool so draws can never starve them.
pub(super) const PASS_SPANS: u32 = 64;
/// Per-draw spans per frame; a frame that needs more drops its draw categories.
pub(super) const DRAW_SPANS: u32 = 448;
pub(super) const SLOT_SPANS: u32 = PASS_SPANS + DRAW_SPANS;
pub(super) const SLOT_BYTES: u64 = SLOT_SPANS as u64 * 2 * TIMESTAMP_BYTES;
pub(super) const TIMESTAMP_BYTES: u64 = 8;
pub(super) const NO_SLOT: u32 = u32::MAX;

pub(super) const PENDING: u8 = 0;
pub(super) const MAPPED: u8 = 1;
pub(super) const FAILED: u8 = 2;
pub(super) const WAITING_RENDER: u8 = 3;
pub(super) const READY_TO_RESOLVE: u8 = 4;

/// A begin/end query pair; the end index is `begin + 1`.
pub(super) struct Span<'a> {
    pub(super) queries: &'a wgpu::QuerySet,
    pub(super) begin: u32,
}

/// Lock-free span allocation for the frame being recorded, shared by graph threads.
pub(super) struct FrameSpans {
    pub(super) slot: AtomicU32,
    pub(super) passes: AtomicU32,
    pub(super) draws: AtomicU32,
    pub(super) stages: [AtomicU8; SLOT_SPANS as usize],
}

pub(super) struct ReadbackSlot {
    pub(super) buffer: wgpu::Buffer,
    pub(super) state: Arc<AtomicU8>,
    pub(super) passes: u32,
    pub(super) draws: u32,
    pub(super) stages: [RuntimeStage; SLOT_SPANS as usize],
}

#[derive(Resource)]
pub(crate) struct GpuTimestamps {
    pub(super) native_passes: bool,
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    pub(super) slots: [ReadbackSlot; SLOTS],
    pub(super) ring: ReadbackRing,
    pub(super) period_ns: f32,
    pub(super) draw_spans: bool,
    pub(super) frame: FrameSpans,
    health: Option<QueryHealth>,
}

impl GpuTimestamps {
    /// `None` when the device lacks `TIMESTAMP_QUERY`.
    pub(super) fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        profiling: bool,
        backend: wgpu::Backend,
    ) -> Option<Self> {
        let features = device.features();
        if !features.contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let device = device.wgpu_device();
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: SLOT_BYTES,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(Self {
            health: QueryHealth::requested(),
            native_passes: backend == wgpu::Backend::Metal,
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("gpu timestamps"),
                ty: wgpu::QueryType::Timestamp,
                count: SLOTS as u32 * SLOT_SPANS * 2,
            }),
            resolve: buffer(
                "gpu timestamp resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            slots: std::array::from_fn(|_| ReadbackSlot {
                buffer: buffer(
                    "gpu timestamp readback",
                    wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                ),
                state: Arc::new(AtomicU8::new(PENDING)),
                passes: 0,
                draws: 0,
                stages: [RuntimeStage::GpuFrame; SLOT_SPANS as usize],
            }),
            ring: ReadbackRing::default(),
            period_ns: queue.get_timestamp_period(),
            draw_spans: profiling
                && features.contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES),
            frame: FrameSpans {
                slot: AtomicU32::new(NO_SLOT),
                passes: AtomicU32::new(0),
                draws: AtomicU32::new(0),
                stages: std::array::from_fn(|_| AtomicU8::new(0)),
            },
        })
    }

    /// Reserves one graph-node or whole-render-pass timestamp pair.
    pub(super) fn open_pass(&self, stage: RuntimeStage) -> Option<Span<'_>> {
        self.open(stage, &self.frame.passes, 0, PASS_SPANS)
    }

    /// Reserves an in-pass pair only when the device and explicit profiler allow it.
    pub(super) fn open_draw(&self, stage: RuntimeStage) -> Option<Span<'_>> {
        self.draw_spans
            .then(|| self.open(stage, &self.frame.draws, PASS_SPANS, DRAW_SPANS))
            .flatten()
    }

    /// Drops spans beyond their fixed pool without allocating or delaying rendering.
    fn open(
        &self,
        stage: RuntimeStage,
        counter: &AtomicU32,
        offset: u32,
        capacity: u32,
    ) -> Option<Span<'_>> {
        let slot = self.frame.slot.load(Ordering::Acquire);
        if slot == NO_SLOT {
            return None;
        }
        let index = counter.fetch_add(1, Ordering::Relaxed);
        if index >= capacity {
            return None;
        }
        let span = offset + index;
        self.frame.stages[span as usize].store(stage as u8, Ordering::Relaxed);
        Some(Span {
            queries: &self.queries,
            begin: (slot * SLOT_SPANS + span) * 2,
        })
    }

    /// Hands mapped frames to `sink` oldest first, then claims a slot for this frame.
    pub(super) fn begin(&mut self, mut sink: impl FnMut(&GpuFrameTimes)) {
        while let Some(index) = self.ring.oldest_in_flight() {
            let slot = &self.slots[index];
            match slot.state.load(Ordering::Acquire) {
                PENDING | WAITING_RENDER | READY_TO_RESOLVE => break,
                MAPPED => {
                    if let Some(health) = &mut self.health {
                        health.readback();
                    }
                    let bytes = slot.buffer.slice(..).get_mapped_range();
                    let tick = |query: u32| {
                        let start = query as usize * TIMESTAMP_BYTES as usize;
                        u64::from_le_bytes(
                            bytes[start..start + TIMESTAMP_BYTES as usize]
                                .try_into()
                                .expect("timestamp is eight bytes"),
                        )
                    };
                    let passes = (0..slot.passes).map(|span| span * 2);
                    let draws = (0..slot.draws).map(|span| (PASS_SPANS + span) * 2);
                    let mut frame = decode_spans(
                        passes.chain(draws).map(|query| {
                            let stage = slot.stages[query as usize / 2];
                            let (begin, end) = (tick(query), tick(query + 1));
                            if let Some(health) = &mut self.health {
                                health.sample(stage, begin, end);
                            }
                            (stage, begin, end)
                        }),
                        self.period_ns,
                    );
                    if self.native_passes {
                        frame.clear_frame_total();
                    }
                    drop(bytes);
                    slot.buffer.unmap();
                    sink(&frame);
                }
                _ => {
                    if let Some(health) = &mut self.health {
                        health.map_failure();
                    }
                }
            }
            slot.state.store(PENDING, Ordering::Relaxed);
            self.ring.release(index);
        }
        let slot = self.ring.acquire().map_or(NO_SLOT, |slot| slot as u32);
        if let Some(health) = &mut self.health {
            health.frame(slot == NO_SLOT);
        }
        self.frame.passes.store(0, Ordering::Relaxed);
        self.frame.draws.store(0, Ordering::Relaxed);
        self.frame.slot.store(slot, Ordering::Release);
    }

    /// Metal counters become visible after render completion; callbacks never submit GPU work.
    pub(super) fn resolve_ready(&mut self, device: &RenderDevice, queue: &RenderQueue) {
        if !self.native_passes {
            return;
        }
        for index in 0..SLOTS {
            if self.slots[index].state.load(Ordering::Acquire) == READY_TO_RESOLVE {
                self.resolve_slot(device, queue, index);
            }
        }
    }

    /// Retains each slot until its asynchronous readback finishes, without waiting on rendering.
    pub(super) fn submit(&mut self, device: &RenderDevice, queue: &RenderQueue) {
        let slot = self.frame.slot.swap(NO_SLOT, Ordering::AcqRel);
        if slot == NO_SLOT {
            return;
        }
        let passes = self.frame.passes.load(Ordering::Relaxed).min(PASS_SPANS);
        let draws = self.frame.draws.load(Ordering::Relaxed);
        let draws = if draws > DRAW_SPANS { 0 } else { draws };
        let index = slot as usize;
        if passes == 0 && draws == 0 {
            self.ring.release(index);
            return;
        }
        let target = &mut self.slots[index];
        target.passes = passes;
        target.draws = draws;
        for span in (0..passes).chain(PASS_SPANS..PASS_SPANS + draws) {
            let stage = self.frame.stages[span as usize].load(Ordering::Relaxed);
            target.stages[span as usize] = RuntimeStage::ALL[stage as usize];
        }
        if self.native_passes {
            target.state.store(WAITING_RENDER, Ordering::Release);
            let state = target.state.clone();
            queue.on_submitted_work_done(move || {
                state.store(READY_TO_RESOLVE, Ordering::Release);
            });
        } else {
            self.resolve_slot(device, queue, index);
        }
        self.ring.submit(index);
    }

    /// Resolves only a reserved slot and maps its copy; Metal callers first await its render callback.
    fn resolve_slot(&mut self, device: &RenderDevice, queue: &RenderQueue, index: usize) {
        let target = &mut self.slots[index];
        target.state.store(PENDING, Ordering::Release);
        let base = index as u32 * SLOT_SPANS * 2;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu timestamp readback"),
        });
        if target.passes > 0 {
            encoder.resolve_query_set(
                &self.queries,
                base..base + target.passes * 2,
                &self.resolve,
                0,
            );
        }
        if target.draws > 0 {
            let first = base + PASS_SPANS * 2;
            encoder.resolve_query_set(
                &self.queries,
                first..first + target.draws * 2,
                &self.resolve,
                u64::from(PASS_SPANS) * 2 * TIMESTAMP_BYTES,
            );
        }
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &target.buffer, 0, SLOT_BYTES);
        queue.submit([encoder.finish()]);
        let state = target.state.clone();
        target
            .buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FAILED },
                    Ordering::Release,
                );
            });
    }
}
