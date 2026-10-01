# Builds the unsigned Windows payload and MSI. Requires: WiX v5 (`dotnet tool install --global wix --version 5.0.2`) and ImageMagick.
# Usage: build-installer.ps1 [-Out <dir>]. Env: CINNABAR_UPDATE_URL (optional).
param([string]$Out = ".local/dist/windows-release")
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "../..")
# WiX resolves relative harvest paths beneath INSTALLFOLDER, so its inputs must be absolute.
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
$release = Join-Path $root "target/release"
foreach ($name in "bedrock-client.exe", "bedrock-core.exe", "bedrock-local-server.exe", "assetc.exe") {
    if (-not (Test-Path (Join-Path $release $name))) { throw "missing $release\$name; run make package-binaries" }
}
$version = (Select-String -Path (Join-Path $root "Cargo.toml") -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value

$payload = Join-Path $Out "payload"
Remove-Item -Recurse -Force $payload -ErrorAction SilentlyContinue
$resources = Join-Path $payload "resources"
$kit = Join-Path $resources "prep-kit"
New-Item -ItemType Directory -Force (Join-Path $resources "assets"), (Join-Path $resources "licenses"), (Join-Path $resources "fonts"), (Join-Path $kit "bin"), (Join-Path $kit "scripts"), (Join-Path $kit "assets"), (Join-Path $kit "data") | Out-Null
Copy-Item (Join-Path $release "bedrock-client.exe"), (Join-Path $release "bedrock-core.exe"), (Join-Path $release "bedrock-local-server.exe") $payload
Copy-Item (Join-Path $release "assetc.exe") (Join-Path $kit "bin")
Copy-Item (Join-Path $root "crates/assets/data/block-physics-v2193.bin") (Join-Path $resources "assets")
Copy-Item (Join-Path $root "THIRD_PARTY_NOTICES.md") (Join-Path $resources "assets")
Copy-Item (Join-Path $root "assets/licenses/*") (Join-Path $resources "licenses")
$fontSource = Get-Content -Raw (Join-Path $root "assets/ui-font-source.json") | ConvertFrom-Json
$font = Join-Path $root ".local/assets/ui-font/$($fontSource.commit)/$($fontSource.font_file)"
if (-not (Test-Path $font)) { throw "missing $font; run scripts/fetch-ui-font.ps1" }
Copy-Item $font (Join-Path $resources "fonts")
foreach ($name in "fetch-vanilla-assets.ps1", "fetch-ui-font.ps1") { Copy-Item (Join-Path $root "scripts/$name") (Join-Path $kit "scripts") }
Copy-Item (Join-Path $root "assets/*.json") (Join-Path $kit "assets")
foreach ($stem in "block-registry", "block-light-registry", "biome-registry") { Copy-Item (Join-Path $root "crates/assets/data/$stem-v2193.*") (Join-Path $kit "data") }
if ($env:CINNABAR_UPDATE_URL) { [System.IO.File]::WriteAllText((Join-Path $resources "update-url"), $env:CINNABAR_UPDATE_URL) }

$icon = Join-Path $Out "cinnabar.ico"
magick -background none -density 384 (Join-Path $PSScriptRoot "../icons/cinnabar.svg") -define icon:auto-resize=256,128,64,48,32,16 $icon

& (Join-Path $PSScriptRoot "sign.ps1") -Path (Join-Path $payload "bedrock-client.exe"), (Join-Path $payload "bedrock-core.exe"), (Join-Path $payload "bedrock-local-server.exe"), (Join-Path $kit "bin/assetc.exe")
$msi = Join-Path $Out "Cinnabar-$version-x64.msi"
wix build -arch x64 -d Version=$version -d Payload=$payload -d Icon=$icon -o $msi (Join-Path $PSScriptRoot "cinnabar.wxs")
if ($LASTEXITCODE -ne 0) { throw "wix build failed" }
& (Join-Path $PSScriptRoot "sign.ps1") -Path $msi
Write-Output $msi
