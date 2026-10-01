# Authenticode-signs files when WINDOWS_CERT_PFX_BASE64 and WINDOWS_CERT_PASSWORD are set; otherwise leaves them unsigned.
param([Parameter(Mandatory)][string[]]$Path)
$ErrorActionPreference = "Stop"
if (-not $env:WINDOWS_CERT_PFX_BASE64) { Write-Warning "no signing certificate configured; $($Path.Count) file(s) left unsigned"; return }
$signtool = (Get-Command signtool -ErrorAction SilentlyContinue).Source
if (-not $signtool) {
    $signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" -ErrorAction SilentlyContinue |
        Sort-Object FullName -Descending | Select-Object -First 1 -ExpandProperty FullName
}
if (-not $signtool) { throw "signtool.exe not found on PATH or under Windows Kits" }
$pfx = Join-Path ([IO.Path]::GetTempPath()) ("cinnabar-" + [guid]::NewGuid() + ".pfx")
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX_BASE64))
try {
    foreach ($file in $Path) {
        & $signtool sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /f $pfx /p $env:WINDOWS_CERT_PASSWORD $file
        if ($LASTEXITCODE -ne 0) { throw "signtool failed for $file" }
    }
} finally { Remove-Item -Force $pfx -ErrorAction SilentlyContinue }
