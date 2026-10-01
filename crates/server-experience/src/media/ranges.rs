//! Seekable authenticated ranges. Use only from a media worker, never rendering or audio.

use std::{collections::{BTreeSet, VecDeque}, io::{self, Read, Seek, SeekFrom}, sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}}};
use anyhow::{Result, ensure};
use super::{MAX_COMPRESSED_BYTES, descriptor::Descriptor};

pub struct RangeReader {
    descriptor: Descriptor,
    origins: BTreeSet<String>,
    runtime: tokio::runtime::Runtime,
    cancelled: Arc<AtomicBool>,
    position: u64,
    chunks: VecDeque<(usize, Vec<u8>)>,
    data_budget: Arc<AtomicU64>,
}

impl RangeReader {
    /// Creates a small cache without starting a request; authority is supplied by the host.
    pub fn new(descriptor: Descriptor, origins: BTreeSet<String>, cancelled: Arc<AtomicBool>, data_budget: Arc<AtomicU64>) -> Result<Self> {
        descriptor.validate(&origins)?;
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
        Ok(Self { descriptor, origins, runtime, cancelled, position: 0, chunks: VecDeque::new(), data_budget })
    }

    /// Authenticates each full chunk before exposing even one byte to the demuxer.
    fn chunk(&mut self, index: usize) -> Result<&[u8]> {
        ensure!(!self.cancelled.load(Ordering::Acquire), "media cancelled");
        let existing = self.chunks.iter().position(|(number, _)| *number == index);
        if let Some(existing) = existing {
            let chunk = self.chunks.remove(existing).expect("existing chunk");
            self.chunks.push_front(chunk);
        } else {
            let start = index as u64 * u64::from(self.descriptor.chunk_bytes);
            let end = (start + u64::from(self.descriptor.chunk_bytes)).min(self.descriptor.bytes) - 1;
            let length = end - start + 1;
            ensure!(self.data_budget.fetch_update(Ordering::AcqRel, Ordering::Acquire,
                |remaining| remaining.checked_sub(length)).is_ok(), "media data allowance exhausted");
            let expected = self.descriptor.chunk_hashes.get(index).ok_or_else(|| anyhow::anyhow!("range outside signed index"))?;
            let download = crate::fetch::fetch(&self.descriptor.url, &self.origins, length as usize, Some((start, end, self.descriptor.bytes)));
            let cancelled = Arc::clone(&self.cancelled);
            let bytes = self.runtime.block_on(async {
                tokio::select! {
                    result = download => result,
                    _ = cancellation(cancelled) => anyhow::bail!("media cancelled"),
                }
            })?;
            ensure!(crate::crypto::digest(&bytes) == *expected, "media range hash mismatch");
            let max_chunks = MAX_COMPRESSED_BYTES / self.descriptor.chunk_bytes as usize;
            while self.chunks.len() >= max_chunks { self.chunks.pop_back(); }
            self.chunks.push_front((index, bytes));
        }
        Ok(&self.chunks.front().expect("chunk loaded").1)
    }
}

impl Read for RangeReader {
    /// Reads at most the current verified chunk; ordinary Read callers retry as needed.
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position == self.descriptor.bytes { return Ok(0); }
        let index = (self.position / u64::from(self.descriptor.chunk_bytes)) as usize;
        let offset = (self.position % u64::from(self.descriptor.chunk_bytes)) as usize;
        let chunk = self.chunk(index).map_err(io::Error::other)?;
        let length = output.len().min(chunk.len() - offset);
        output[..length].copy_from_slice(&chunk[offset..offset + length]);
        self.position += length as u64;
        Ok(length)
    }
}

impl Seek for RangeReader {
    /// Rejects invalid offsets rather than allowing wraparound or sparse allocation.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::End(delta) => i128::from(self.descriptor.bytes) + i128::from(delta),
            SeekFrom::Current(delta) => i128::from(self.position) + i128::from(delta),
        };
        if next < 0 || next > i128::from(self.descriptor.bytes) { return Err(io::Error::other("media seek outside object")); }
        self.position = next as u64;
        Ok(self.position)
    }
}

/// Interrupts a pending request after revocation without a worker join on the main thread.
async fn cancellation(cancelled: Arc<AtomicBool>) {
    while !cancelled.load(Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}
