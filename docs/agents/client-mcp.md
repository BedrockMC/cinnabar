# Driving and recording the client (cinnabar-mcp)

`tools/cinnabar-mcp` is a stdio MCP server that launches a client built with
`cargo build -p bedrock-client --features developer-control` (plus `local-mods` for mods) and drives
it over a loopback, token-authenticated endpoint written under `.local/developer-control/`. The tool
schemas document every argument.

- `connect` with `local_server` runs `make local-server`'s binary on a free loopback port. Remote
  joins need `allow_remote`, since they sign in with the configured account; ask the owner first.
- `input` sends real key and mouse events, so mods' ability keys (`Digit1`…) work; `F1` hides the HUD.
- `record_start` needs `ffmpeg` on PATH. Its default fixed clock steps game time exactly 1/fps per
  rendered frame, so it suits the local showcase server; record remote servers with
  `fixed_clock: false`. Audio is captured to a WAV and muxed in.

```json
{ "mcpServers": { "cinnabar": { "command": "target/debug/cinnabar-mcp", "args": ["--repo", "."] } } }
```
