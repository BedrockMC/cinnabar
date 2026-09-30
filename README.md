# Cinnabar

> An independent, unofficial client compatible with Minecraft: Bedrock Edition. Not approved by
> or associated with Mojang or Microsoft. Minecraft is a trademark of Microsoft Corporation.

A Bedrock client written in Rust (Bevy/wgpu), targeting vanilla parity with the release pinned
in `assets/bedrock-target.json`. A small Go core handles Microsoft sign-in and upstream
networking.

<img width="2534" height="1446" alt="Cinnabar in game" src="https://github.com/user-attachments/assets/836cb337-3876-4b31-a97e-9cfb25227b11" />

## Play

```sh
make play
```

This downloads and compiles the vanilla assets on first run (and whenever they're stale), builds
the Go core, and opens the launcher menu. The first sign-in prints a Microsoft device code; the
token is cached in `.local/auth/`, which holds private credentials, so never share or commit it.

To join one server directly without the menu, run the core and client in two terminals:

```sh
make core UPSTREAM=zeqa.net:19132
make client
```

`make help` lists every target. On Debian/Ubuntu, install `libwayland-dev` first; Linux picks
Wayland or X11 automatically.

## How it fits together

```text
bedrock-client (Rust)  ── local socket ──  bedrock-core (Go, gophertunnel)
                                             ├─ go-raknet ──── servers and BDS
                                             └─ go-nethernet ─ Realms and friend worlds
```

Rust never implements Xbox authentication, encryption, RakNet or NetherNet; the core owns those
and relays packets over a local stream.

| Library | Used for |
| --- | --- |
| [protocolgen](https://github.com/bedrock-mc/protocolgen) | Generates the Bedrock packet definitions behind `crates/protocol`. |
| [Axolotl Stack](https://github.com/axolotl-stack/axolotl-stack) | Valentine (packet codec) and Jolyne (client transport), vendored in `crates/protocol/vendor`. |
| [gophertunnel](https://github.com/Sandertv/gophertunnel) | Bedrock login, encryption, resource packs and the packet relay. |
| [go-raknet](https://github.com/Sandertv/go-raknet) | RakNet transport to servers, plus server-list pings. |
| [go-nethernet](https://github.com/df-mc/go-nethernet) | WebRTC transport for Realms and friend worlds. |
| [go-xsapi](https://github.com/df-mc/go-xsapi) | Xbox Live identity, friends, presence and signaling. |
| [go-playfab](https://github.com/df-mc/go-playfab) | PlayFab sign-in and the menu catalog (featured servers, marketplace). |
| [dragonfly](https://github.com/df-mc/dragonfly) | The built-in local-world server in `tools/localserver`. |

Mojang assets are never committed or embedded. `make assets` fetches Mojang's official
`bedrock-samples` pack (EULA-gated) and compiles it into carriers under the ignored `.local/`.

## Workspace

| Crate | What it does |
| --- | --- |
| `app` | The `bedrock-client` binary: Bevy app, networking glue, gameplay, menus and HUD. |
| `crates/asset-compiler` | `assetc`, which compiles the vanilla pack into the runtime carriers. |
| `crates/assets` | Readers for pack sources and compiled carriers. |
| `crates/bridge` | The local stream between the client and the Go core. |
| `crates/client-world` | Client game state: actors, items, block entities and the packet stream. |
| `crates/input` | Device-independent input actions. |
| `crates/json-ui` | Parser, resolver and layout engine for vanilla JSON-UI. |
| `crates/meshing` | CPU geometry for chunks, liquids, biomes and clouds. |
| `crates/protocol` | Bedrock packet definitions and codec. |
| `crates/render` | Chunk and entity rendering on Bevy/wgpu. |
| `crates/resource-pack` | Admission and decryption of server resource packs. |
| `crates/sim` | Deterministic Bedrock movement simulation. |
| `crates/ui` | Renderer-independent UI primitives and text layout. |
| `crates/world` | Palette-native chunk and world model. |
| `tools/architecture` | Architecture gate: line limits, dependency rules, markers. |
| `tools/devtool` | `verify-affected`, which tests only what a change touches. |
| `tools/dist` | Stages distributable bundles. |
| `tools/phase2-evidence`, `tools/visualcoverage` | Frozen evidence replays from earlier milestones. |

| Go package (`core/`) | What it does |
| --- | --- |
| `cmd/bedrock-core` | The core binary. |
| `proxy` | Upstream session, resource-pack download and packet relay. |
| `authflow`, `authcache` | Microsoft device sign-in and token cache. |
| `catalog`, `store`, `launcher`, `control` | Menu data: featured servers, Realms, friends, marketplace. |
| `localworld` | Local worlds on BDS (a container on macOS). |
| `packcache` | On-disk cache of server packs. |
| `update`, `crashreport` | Signed update checks and crash reports. |

## Development

Before pushing, run what CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo run -p architecture -- check --root . --policy tools/architecture/policy.toml
cargo test --workspace --locked
go test ./core/...
```

`cargo run -p devtool --locked -- verify-affected --base origin/main` runs only the affected
packages. Contributor and agent rules live in `AGENTS.md` and `docs/agents/`.
