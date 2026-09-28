# Local worlds

Single-player worlds run on dragonfly behind the same core and game socket as online play.
Full scope: `plan.md` Phase 7.

- **Host:** `tools/localserver` builds `bedrock-local-server` (`make local-server`), one child process per
  open world. It is a standalone module (`GOWORK=off`) because dragonfly needs a newer gophertunnel than the
  core's pin; the two never share a process or a `go.work`.
- **Core:** `-control-status -local-worlds-dir <dir>` enables `core/localworld` and the `world_*.v1`
  control methods. `-local-server-bin` overrides the binary (default: beside the core).
- **Routing:** with a world open the proxy dials its loopback address; otherwise it uses `-upstream`.
- **Storage:** `<dir>/<id>/world.json` (settings) plus `db/` (LevelDB) and `players/`. Reopening restores both.
- **Pause:** the client sends `world_pause.v1` on window focus loss/regain while playing; the host freezes
  time and block/entity ticking, players stay connected.
- **Client:** `app/src/local_worlds` is the embeddable screen model and control worker; the menu module drives it.

## v1 limits

Terrain is a seeded value-noise approximation (`normal`) or superflat, not vanilla worldgen; mob AI and
other dragonfly parity gaps are accepted. Player data persistence depends on the client's offline identity
staying stable across sessions.
