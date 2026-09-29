# Local worlds

Single-player worlds run on a local server behind the same core and game socket as online play.
Full scope: `plan.md` Phase 7.

## Backends

Each world records the backend that created it (`world.json`) and always reopens on it; switching is never silent.

| Backend | Where | Notes |
| --- | --- | --- |
| `bds` native | Windows and Linux x86-64 | Official Bedrock Dedicated Server: vanilla terrain and mobs. |
| `bds` container | macOS (or any host without a native build) with a Docker-compatible runtime | Linux BDS in `itzg/minecraft-bedrock-server`, `--platform linux/amd64`. |
| `dragonfly` | fallback when neither can run, and worlds created on it | Simpler terrain, no vanilla mob behavior. |

New worlds default to BDS when it can run (`-local-backend=auto`), else dragonfly; `world_create.v1` may pass `backend`.
`docker_missing` / `docker_not_running` are reported as `backend_unavailable_reason` in status.

## BDS acquisition

Never bundled or committed. `-bds-dir` (default `bds/` beside the worlds dir) holds `<version>/` builds with a
`manifest.json` (URL, zip SHA-256, size, platform, time).

- The target build is `localworld.TargetVersionPrefix` (bump with the client protocol); the download API's current
  build must match it, or `-bds-version` names an exact build fetched from its versioned official URL.
- Only https `minecraft.net` / `minecraft-services.net` hosts (including redirects) are accepted; zips are
  size-capped and unpacked with path-escape checks. Mojang publishes no hash, so the SHA-256 is provenance, not a pin.
- The EULA gate: `world_open.v1` on a BDS world fails with code -32012 until `bds_accept_eula.v1 {"accepted":true}`;
  nothing is downloaded before then. Status carries `setup` (state, bytes, runtime, reason, `eula_accepted`).
- Container runtime: the image downloads the same official build inside the container (`VERSION`, `EULA=TRUE` only
  after in-app acceptance). Pin `DefaultBDSImage` to a digest at release.

## Runtime behavior

- **Storage:** `<worlds>/<id>/world.json` plus `db/` (a Bedrock world folder: `level.dat` + LevelDB) and `players/`.
  BDS sees `db/` through a directory link at `worlds/<id>` (junction on Windows) or a bind mount, so one world
  runs at a time. Worlds move between backends only where formats allow; dragonfly-written `level.dat` files may
  not satisfy BDS and vice versa, so do not share one folder across backends.
- **server.properties / env:** name, gamemode, difficulty, seed, level type, `online-mode=false`, `max-players=1`,
  view and tick distance from the client's `view_distance` (5-32). Seed and level type apply at creation only.
- **Exposure:** BDS cannot bind loopback only; it listens on all interfaces on a random port, offline, one slot.
  The container maps its port to `127.0.0.1` only.
- **Lifecycle:** ready on "Server started."; stop is `docker stop` (container) or `stop` on stdin, then kill after 30 s.
- **Pause:** dragonfly freezes time and block/entity ticking on focus loss. BDS has no equivalent of the
  single-player pause; freezing daylight or weather gamerules is not a pause and persists into `level.dat`, so BDS
  worlds keep running and status reports `pause_supported: false`.
- **Login:** the core dials without a Microsoft session for local play (offline chain from the client's identity);
  BDS accepts it because `online-mode=false`. Player-data persistence needs a stable client identity.

## Control methods

`world_list/create/rename/delete/open/close/pause/status.v1`, `bds_accept_eula.v1`, and `local_worlds_prefs.v1`
(`docker_prompt_dismissed`, `redetect`), all schema v1.

## Client

`app/src/local_worlds` is the embeddable screen model (list, create, delete, rename, open, EULA, Docker modal)
and control worker; the menu and JSON-UI screens bind to it.

## v1 limits

dragonfly terrain is a seeded value-noise approximation or superflat; mob AI and other dragonfly parity gaps are
accepted. Docker-mounted LevelDB on macOS can be slow. A killed core can leave a container running; the next start
of that world removes it.
