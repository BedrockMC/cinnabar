# Packaging

Release installers ship the client, the Go core, the dragonfly local-world server beside it, and a
prep kit. They never ship Mojang-derived carriers: on first launch `app/src/first_run` asks consent,
fetches the pinned public `bedrock-samples` pack (`assets/vanilla-source.json`, hash-verified), runs
the bundled `assetc`, and publishes carriers to the per-user data directory
(`InstallLayout::prepared_assets_dir`). Status: `logs/first-run-status.json`; details:
`logs/first-run.log`. On macOS and Linux a launch without a terminal (e.g. from Finder) sends the
client's stderr to `logs/client.log`, rotated to `client.log.1` per launch and past 8 MiB.

| Target | Command | Signing (env only) |
| --- | --- | --- |
| macOS `.app` + DMG | `make package-macos` | `CODESIGN_IDENTITY`, `NOTARY_PROFILE` or `APPLE_ID`/`APPLE_TEAM_ID`/`APPLE_APP_PASSWORD` |
| Windows MSI (WiX v4) | `make package-windows` | `WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD` |
| Linux AppImage | `make package-linux` | none |

CI: `.github/workflows/package.yml` (tag `v*`). Version comes from `[workspace.package]`.

Without `CODESIGN_IDENTITY` the macOS app is ad-hoc signed and not notarized. Gatekeeper then blocks
it on other Macs until the recipient runs `xattr -dr com.apple.quarantine /Applications/Cinnabar.app`
(or uses System Settings > Privacy & Security > Open Anyway).

## Sign-in
The core owns Xbox device-code auth. The client's `AuthState::AwaitingCode { uri, code }` exposes the
code and URL; no packaging-specific UI exists.

## Crash reports
A panic hook writes `crashes/*.json` (message, backtrace, core log tail) for local debugging; the newest
reports are kept and nothing is uploaded.

## Update channel
`bedrock-core check-update` fetches an Ed25519-signed manifest (`core/update`), rejects unknown keys,
expiry, wrong channel, non-HTTPS, and bad digests, and the client records the verdict in
`update/available.json` at most daily. Manifest URL: `CINNABAR_UPDATE_URL` or `resources/update-url`.
Trusted keys are baked in with `-X main.trustedUpdateKeys=id:base64[,...]` (`UPDATE_TRUSTED_KEYS`),
so rotation ships as a new key ID. CI signs with `core/cmd/release-manifest` using
`CINNABAR_UPDATE_SIGNING_KEY`. Installing an update is manual (download the listed artifact).
