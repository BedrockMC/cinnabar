use crate::chunk::*;

/// Arena writes collected while GPU preparation admits uploads.
#[derive(Default)]
pub(in crate::chunk) struct ArenaWrites {
    pub(in crate::chunk) quads: Vec<(u32, Vec<[u32; 2]>)>,
    pub(in crate::chunk) model: Vec<(u32, Vec<[u32; 4]>)>,
    pub(in crate::chunk) model_lighting: Vec<(u32, Vec<[u16; 4]>)>,
    pub(in crate::chunk) model_draw: Vec<(u32, Vec<[u32; 2]>)>,
    pub(in crate::chunk) transparent_model_draw: Vec<(u32, Vec<[u32; 2]>)>,
    pub(in crate::chunk) liquid: Vec<(u32, Vec<[u32; 4]>)>,
    pub(in crate::chunk) liquid_lighting: Vec<(u32, Vec<[u16; 4]>)>,
    pub(in crate::chunk) cube_lighting: Vec<(u32, Vec<[u16; 4]>)>,
    pub(in crate::chunk) biome: Vec<(u32, Vec<u32>)>,
    pub(in crate::chunk) origins: Vec<(u32, GpuChunkOrigin)>,
}

impl ArenaWrites {
    /// Every admitted upload writes an origin record.
    pub(in crate::chunk) fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    /// Stages the collected writes into the arena's current buffers. The next
    /// queue submit runs them first, so they must be issued before a
    /// migration slice copies the regions they touch.
    pub(in crate::chunk) fn issue(&mut self, arena: &ChunkGpuArena, render_queue: &RenderQueue) {
        write_stream_records(
            render_queue,
            &arena.quad_buffer,
            PACKED_QUAD_BYTES,
            std::mem::take(&mut self.quads),
        );
        for (index, origin) in self.origins.drain(..) {
            render_queue.write_buffer(
                &arena.origin_buffer,
                u64::from(index) * CHUNK_ORIGIN_BYTES,
                bytemuck::bytes_of(&origin),
            );
        }
        let stream = &arena.geometry_stream_buffer;
        let word = GEOMETRY_STREAM_WORD_BYTES;
        write_stream_records(render_queue, stream, word, std::mem::take(&mut self.model));
        let model_lighting = std::mem::take(&mut self.model_lighting);
        write_stream_records(render_queue, stream, word, model_lighting);
        write_stream_records(
            render_queue,
            stream,
            word,
            std::mem::take(&mut self.model_draw),
        );
        let transparent = std::mem::take(&mut self.transparent_model_draw);
        write_stream_records(render_queue, stream, word, transparent);
        write_stream_records(render_queue, stream, word, std::mem::take(&mut self.liquid));
        let liquid_lighting = std::mem::take(&mut self.liquid_lighting);
        write_stream_records(render_queue, stream, word, liquid_lighting);
        let cube_lighting = std::mem::take(&mut self.cube_lighting);
        write_stream_records(render_queue, stream, word, cube_lighting);
        write_stream_records(
            render_queue,
            &arena.biome_buffer,
            BIOME_WORD_BYTES,
            std::mem::take(&mut self.biome),
        );
    }
}
