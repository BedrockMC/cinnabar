# Control channel

`-control-status` binds a local JSON-RPC 2.0 endpoint (4-byte big-endian length frames, one request per
connection). Every result carries `schema_version: 1`. Errors are sanitized: fixed messages, never paths or
upstream text. Rust clients: `crates/bridge`, re-exported by `protocol::launcher_control` and
`protocol::world_control`.

| Method | Params | Result |
| --- | --- | --- |
| `status.v1` | none | lifecycle, pack admission, optional `transfer` |
| `pack_application.v1` | `attempt_id`, `applied` | status |
| `world_*.v1` | see `docs/local-worlds.md` | worlds / status |
| `realms_list.v1` | none | `realms: [{name, state, target, address?}]` |
| `friends_list.v1` | none | `friends: [{gamertag, xuid, world_name, members, max_members, handle_id?, address?}]` |
| `connect.v1` | `kind` = `raknet` (`host:port`), `realm` (id), `friend` (xuid); `value` | empty; next client connection dials it |
| `account_status.v1` | none | `account` |
| `sign_out.v1` | none | `account`; deletes cached tokens |
| `events.v1` | none | `auth`, `disconnect?`, `transfer?` |

`account` / `auth`: `state` is `offline | signed_out | awaiting_code | signed_in | failed`, with
`verification_uri` and `user_code` while awaiting a code, `gamertag` when signed in, `reason` on failure.
`disconnect`: `{reason, message, sequence}` and `transfer`: `{host, port, sequence}` stay until the next
connection attempt begins; poll and compare `sequence`.

Errors: `-32020` not signed in, `-32021` service unavailable, `-32022` invalid target, `-32023` launcher services
disabled, `-32602` invalid params. Precedence for the next connection: pending server transfer, `connect.v1`
selection, open local world, `-upstream`. `world_open.v1` clears the selection and any pending transfer.

Limits: after `sign_out.v1` the running core stops using the account; signing in again needs the device-code
flow (`-auth-events`) and a core restart. Catalog calls can take tens of seconds; requests are served concurrently.
