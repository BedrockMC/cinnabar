# Authenticode-signs files when WINDOWS_CERT_PFX_BASE64 and WINDOWS_CERT_PASSWORD are set; otherwise leaves them unsigned.
param([Parameter(Mandatory)][string[]]$Path)
$ErrorActionPreference = "Stop"
if (-not $env:WINDOWS_CERT_PFX_BASE64) { Write-Warning "no signing certificate configured; $($Path.Count) file(s) left unsigned"; return }
$pfx = Join-Path ([IO.Path]::GetTempPath()) ("cinnabar-" + [guid]::NewGuid() + ".pfx")
[IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_CERT_PFX_BASE64))
try {
    foreach ($file in $Path) {
        signtool sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /f $pfx /p $env:WINDOWS_CERT_PASSWORD $file
        if ($LASTEXITCODE -ne 0) { throw "signtool failed for $file" }
    }
} finally { Remove-Item -Force $pfx -ErrorAction SilentlyContinue }
