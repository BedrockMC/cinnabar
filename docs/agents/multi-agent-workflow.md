# Agent build, cache, and verification discipline

The multi-agent process ceremony (model routing, writer-count caps, mandatory
review loops) has been removed — use as many subagents as help, in whatever
shape fits. Only the mechanical build/cache/verify rules below remain, because
they still catch real breakage regardless of model.

## Concurrent compiles: cap to machine capacity

Any number of agents may read, reason, and edit in parallel — but cap the number
of `cargo` compiles (build/test/clippy) running at once across all worktrees, or
Rust build contention drives the machine load into the range where agents stall.
This machine (an M3 Pro) handles **4** concurrent cold builds; use fewer on
weaker hardware. Enforce it with a shared N-slot build semaphore that every heavy
cargo invocation passes through (a macOS-portable one uses an atomic `mkdir` lock
over N slot dirs with dead-PID reclaim; `flock` is not on macOS; `CBUILD_SLOTS`
tunes N). Editing is unbounded; only the compile step is gated.

## Build cache

- Give each concurrently active worktree its own Cargo `target` directory. A
  shared `CARGO_TARGET_DIR` lets Cargo file locks and path-based fingerprints
  reuse incompatible local crate artifacts across branches.
- Share compiler results through the installed `sccache` (this machine disables
  incremental compilation and caps the cache at 20 GiB).
- Delete a worktree's reproducible `target` after its commit is integrated,
  preserving the canonical checkout's `target/debug/bedrock-client.exe` and any
  actively compiling agent's directory. Use `git worktree`, not another clone.
- A corrupted cached rlib shows up as an `Undefined symbols for architecture
  arm64` link error under load; `cargo clean -p <crate>` for the affected crate
  and rebuild.

## Verify before pushing

CI is a backstop, not a compiler. Before pushing a shared branch, run what the
push will trigger: the focused tests for the changed crates, `cargo fmt --all`,
clippy, and `cargo run -p architecture -- check --root . --policy
tools/architecture/policy.toml`. Run broader suites in proportion to integration
risk. The architecture gate is the one most often skipped and the one that most
often fails — nothing else catches per-file line limits, forbidden test-only
public API, or marker registration.

Capture cargo's real exit code — never let a pipe to `tail` mask it. macOS app
tests that panic with `InvalidMacOsBundle` / `MenuRuntime::discover` from a
non-canonical target dir are an environment artifact, not a failure; they pass
from the canonical `target/`.

When local verification is genuinely unavailable, say the state is unverified and
either wait, hand the run to the user, or push while labeling it unverified in
the same breath. A red CI run must never be the first thing that discovers
whether the code compiles.
