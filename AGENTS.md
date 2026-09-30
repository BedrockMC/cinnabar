# Repository agent instructions

Cinnabar is a Rust Bedrock client plus a Go core. The target is version-matched vanilla
Bedrock parity in every system — UI, rendering, controls, camera, movement, animation,
interaction, inventory, audio, protocol, timing. Base each behavior on an identified vanilla
reference, never on Java Edition, a custom aesthetic, or memory. A provisional approximation
may land only when labeled incomplete in `plan.md`; it never closes a parity gate.

| Load this | When |
| --- | --- |
| `docs/agents/multi-agent-workflow.md` | Worktrees, build limits, verify-before-push |
| `docs/agents/live-testing.md` | Running the client or BDS, capturing frames, closing a visual/performance gate |

## Parity sources and UI


## No hardcoded duplicates

A value that must stay in sync — game/protocol version, pinned pack, registry versions, service
hosts, carrier names, shared sizes — has exactly one source of truth (a constant, manifest, or
generated file) that everything else imports or reads. Never restate it as a literal, including
in UI text and tests.

## Server data: lenient

Malformed framing (truncation, bad lengths, envelope failures) is fatal. Odd but well-formed data
(unknown slot, sentinel id, non-finite float, unknown metadata, custom world height) is skipped,
counted and logged; the session stays up. Chunk payloads follow the vanilla client's lenient
decode exactly. Never disconnect over data the client doesn't use.

## Required carriers: fail closed

Startup requires the atmosphere, entity, HUD and JSON-UI carriers. If one is missing, malformed
or fails its pinned hash, abort from `main` naming the carrier path and its rebuild command
(`make assets`). Never hide required art behind a log line. Exceptions: no world carrier selects
diagnostic textures; no compiled font selects the diagnostic font. New optional carriers degrade
gracefully until startup truly requires them.

## Gophertunnel

Cinnabar's Gophertunnel work lives on `HashimTheArab/gophertunnel:resource-pack-changes` (based on
`lunar`); pin the Go module to a commit on it. Pull `lunar` into it; never push to `lunar` unless
asked.

## Git and payloads

Mojang assets, screenshots, recordings, `.local/` carriers, credentials and BDS binaries never
enter git. Use `git worktree`, one Cargo `target` per active worktree, and delete a worktree's
`target` once its work is integrated.

## Verify before pushing

Before pushing a shared branch run what CI runs: focused tests, `cargo fmt --all`, clippy, and
`cargo run -p architecture -- check --root . --policy tools/architecture/policy.toml` — the gate
most often forgotten (line limits, test-only public API, markers). If local verification is
impossible, say the state is unverified; CI must never be the first compile.

## Report state precisely

Distinguish pushed, locally committed, test-green uncommitted, and in-progress work.
