//! Every resource limit of the Experience runtime. This file is the only source of truth.

use std::time::Duration;

/// Fuel granted to one callback.
pub const CALLBACK_FUEL: u64 = 10_000_000;
/// Fuel granted to `register`.
pub const REGISTER_FUEL: u64 = 100_000_000;
/// Epoch tick period, the wall-clock backstop for fuel.
pub const EPOCH_PERIOD: Duration = Duration::from_millis(1);
/// Wall-clock deadline for one callback.
pub const CALLBACK_DEADLINE: Duration = Duration::from_millis(250);
/// Wall-clock deadline for `register`.
pub const REGISTER_DEADLINE: Duration = Duration::from_secs(2);
/// Linear memory per instance.
pub const MAX_MEMORY_BYTES: usize = 64 * 1024 * 1024;
/// Linear memories per instance.
pub const MAX_MEMORIES: usize = 1;
/// Table elements per instance.
pub const MAX_TABLE_ELEMENTS: usize = 10_000;
/// Core instances per component instance.
pub const MAX_CORE_INSTANCES: usize = 16;
/// Wasm stack.
pub const MAX_WASM_STACK_BYTES: usize = 256 * 1024;
/// Size of `server.wasm`.
pub const MAX_COMPONENT_BYTES: usize = 16 * 1024 * 1024;
/// Blocks one Experience may register.
pub const MAX_BLOCKS: usize = 64;
/// Host calls per callback; logs are counted separately.
pub const MAX_HOST_CALLS: usize = 256;
/// Staged ops per callback.
pub const MAX_STAGED_OPS: usize = 64;
/// Logs per callback or `register`.
pub const MAX_LOGS: usize = 32;
/// Bytes per log line.
pub const MAX_LOG_BYTES: usize = 512;
/// Encoded JSON bytes per IPC frame, excluding the 4-byte length prefix.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
