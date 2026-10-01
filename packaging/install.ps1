# Install the latest stable Cinnabar release. Compatible with Windows PowerShell 5.1.
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'

function Get-CinnabarWindowsArchitecture {
    if ([System.Environment]::OSVersion.Platform -ne [System.PlatformID]::Win32NT) {
        throw 'Cinnabar: this installer requires Windows.'
    }
    $architecture = $env:PROCESSOR_ARCHITEW6432
    if ([string]::IsNullOrWhiteSpace($architecture)) { $architecture = $env:PROCESSOR_ARCHITECTURE }
    if ($architecture -ne 'AMD64') { throw "Cinnabar: unsupported Windows architecture: $architecture (x64 required)." }
    return $architecture
}

function Install-Cinnabar {
    $null = Get-CinnabarWindowsArchitecture
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $releaseRoot = 'https://github.com/bedrock-mc/cinnabar/releases'
    $latest = Invoke-WebRequest -Uri "$releaseRoot/latest" -Method Head -UseBasicParsing
    # Windows PowerShell uses HttpWebResponse; PowerShell 7 uses HttpResponseMessage.
    if ($latest.BaseResponse.PSObject.Properties['ResponseUri']) {
        $resolved = $latest.BaseResponse.ResponseUri.AbsoluteUri
    } else {
        $resolved = $latest.BaseResponse.RequestMessage.RequestUri.AbsoluteUri
    }
    $releasePattern = '^' + [regex]::Escape($releaseRoot) + '/tag/(v[0-9]+\.[0-9]+\.[0-9]+)$'
    if ($resolved -notmatch $releasePattern) { throw "Cinnabar: could not resolve a stable release from $resolved." }
    $tag = $Matches[1]
    $asset = 'Cinnabar-x64.msi'
    $downloadRoot = "$releaseRoot/download/$tag"
    $temporary = Join-Path ([System.IO.Path]::GetTempPath()) ('cinnabar-install-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temporary | Out-Null
    try {
        Write-Output "Downloading Cinnabar $tag (x64)..."
        $checksumPath = Join-Path $temporary 'SHA256SUMS.txt'
        $installerPath = Join-Path $temporary $asset
        Invoke-WebRequest -Uri "$downloadRoot/SHA256SUMS.txt" -OutFile $checksumPath -UseBasicParsing
        Invoke-WebRequest -Uri "$downloadRoot/$asset" -OutFile $installerPath -UseBasicParsing
        $checksumPattern = '^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($asset) + '$'
        $checksums = @(Get-Content -LiteralPath $checksumPath | ForEach-Object {
            if ($_ -match $checksumPattern) { $Matches[1] }
        })
        if ($checksums.Count -ne 1) { throw "Cinnabar: missing or ambiguous SHA-256 checksum for $asset." }
        $actual = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash
        if ($actual -ne $checksums[0]) { throw "Cinnabar: SHA-256 mismatch for $asset." }
        $process = Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', ('"' + $installerPath + '"'), '/passive', '/norestart') -Wait -PassThru
        if ($process.ExitCode -notin @(0, 3010)) { throw "Cinnabar: Windows Installer failed with exit code $($process.ExitCode)." }
        Write-Output "Installed Cinnabar $tag. Launch it from the Start menu to complete first-run setup."
        if ($process.ExitCode -eq 3010) { Write-Output 'Windows Installer requested a restart.' }
    } finally {
        Remove-Item -LiteralPath $temporary -Recurse -Force
    }
}

Install-Cinnabar
