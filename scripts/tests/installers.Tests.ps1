$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$installer = Join-Path $PSScriptRoot '../../packaging/install.ps1'
$tokens = $null
$parseErrors = $null
$syntax = [System.Management.Automation.Language.Parser]::ParseFile($installer, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw ($parseErrors | Out-String) }
# Load only functions so test doubles can exercise installation without running msiexec.
foreach ($definition in $syntax.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($definition.Extent.Text))
}
$nativeArchitecture = (Get-Command Get-CinnabarWindowsArchitecture).ScriptBlock

function Assert-True($condition, [string]$message) {
    if (-not $condition) { throw $message }
}

$script:fixture = Join-Path ([System.IO.Path]::GetTempPath()) ('cinnabar-installer-tests-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $script:fixture | Out-Null
$script:assetPath = Join-Path $script:fixture 'Cinnabar-x64.msi'
[System.IO.File]::WriteAllText($script:assetPath, 'synthetic MSI')
$script:expected = (Get-FileHash -LiteralPath $script:assetPath -Algorithm SHA256).Hash
$script:requests = @()
$script:downloads = @()
$script:started = @()
$script:resultCode = 0
$script:corrupt = $false
$script:duplicate = $false
$script:unsupported = $false
$script:failDownload = $false
$script:tag = 'v9.8.7'
$script:responseStyle = 'Desktop'

function Get-CinnabarWindowsArchitecture {
    if ($script:unsupported) { throw 'Cinnabar: unsupported Windows architecture: ARM64 (x64 required).' }
    return 'AMD64'
}

function Invoke-WebRequest {
    param([string]$Uri, [string]$Method, [string]$OutFile, [switch]$UseBasicParsing)
    $script:requests += $Uri
    if ($Method -eq 'Head') {
        $resolved = [Uri]('https://github.com/bedrock-mc/cinnabar/releases/tag/' + $script:tag)
        if ($script:responseStyle -eq 'Desktop') { return [pscustomobject]@{ BaseResponse = [pscustomobject]@{ ResponseUri = $resolved } } }
        return [pscustomobject]@{ BaseResponse = [pscustomobject]@{ RequestMessage = [pscustomobject]@{ RequestUri = $resolved } } }
    }
    Assert-True ($Uri.Contains('/download/v9.8.7/')) 'downloads were not pinned to the resolved release'
    $script:downloads += $OutFile
    if ($script:failDownload) { throw 'synthetic download failed' }
    if ($Uri.EndsWith('/SHA256SUMS.txt')) {
        $lines = @("$script:expected  Cinnabar-x64.msi")
        if ($script:duplicate) { $lines += $lines[0] }
        Set-Content -LiteralPath $OutFile -Value $lines
    } else {
        Copy-Item -LiteralPath $script:assetPath -Destination $OutFile
        if ($script:corrupt) { Add-Content -LiteralPath $OutFile -Value 'tampered' }
    }
}

function Start-Process {
    param([string]$FilePath, [string[]]$ArgumentList, [switch]$Wait, [switch]$PassThru)
    Assert-True ($FilePath -eq 'msiexec.exe') 'unexpected installer command'
    Assert-True ($Wait -and $PassThru) 'installer did not wait and inspect exit status'
    Assert-True ($ArgumentList[0] -eq '/i' -and $ArgumentList[2] -eq '/passive' -and $ArgumentList[3] -eq '/norestart') 'MSI install arguments changed'
    Assert-True ($ArgumentList[1].StartsWith('"') -and $ArgumentList[1].EndsWith('"')) 'MSI path was not quoted'
    $path = $ArgumentList[1].Trim('"')
    Assert-True (Test-Path -LiteralPath $path) 'downloaded MSI missing during execution'
    $script:started += $path
    return [pscustomobject]@{ ExitCode = $script:resultCode }
}

function Expect-Failure([string]$message) {
    $caught = $null
    try { Install-Cinnabar | Out-Null } catch { $caught = $_.Exception.Message }
    Assert-True ($null -ne $caught -and $caught.Contains($message)) "expected failure '$message', got '$caught'"
}

try {
    if ([System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT) {
        $savedArchitecture = $env:PROCESSOR_ARCHITECTURE
        $savedNativeArchitecture = $env:PROCESSOR_ARCHITEW6432
        try {
            $env:PROCESSOR_ARCHITECTURE = 'x86'
            $env:PROCESSOR_ARCHITEW6432 = 'AMD64'
            Assert-True ((& $nativeArchitecture) -eq 'AMD64') '32-bit PowerShell did not select the native x64 installer'
            $env:PROCESSOR_ARCHITEW6432 = 'ARM64'
            $architectureError = $null
            try { & $nativeArchitecture | Out-Null } catch { $architectureError = $_.Exception.Message }
            Assert-True ($null -ne $architectureError -and $architectureError.Contains('unsupported Windows architecture')) 'native ARM64 was accepted'
        } finally {
            $env:PROCESSOR_ARCHITECTURE = $savedArchitecture
            $env:PROCESSOR_ARCHITEW6432 = $savedNativeArchitecture
        }
    } else {
        $platformError = $null
        try { & $nativeArchitecture | Out-Null } catch { $platformError = $_.Exception.Message }
        Assert-True ($null -ne $platformError -and $platformError.Contains('requires Windows')) 'non-Windows host was accepted'
    }
    foreach ($style in @('Desktop', 'Core')) {
        $script:responseStyle = $style
        Install-Cinnabar | Out-Null
        $lastPath = $script:started[-1]
        Assert-True (-not (Test-Path -LiteralPath (Split-Path -Parent $lastPath))) 'installer left temporary files'
    }
    Assert-True ($script:requests.Count -eq 6 -and $script:started.Count -eq 2) 'expected one release resolution and two downloads per install'
    $script:resultCode = 3010
    $output = Install-Cinnabar | Out-String
    Assert-True ($output.Contains('requested a restart')) 'successful MSI reboot result was not reported'
    $script:resultCode = 1603
    Expect-Failure 'exit code 1603'
    Assert-True (-not (Test-Path -LiteralPath (Split-Path -Parent $script:started[-1]))) 'failed MSI left temporary files'
    $script:resultCode = 0
    $before = $script:started.Count
    $script:corrupt = $true
    Expect-Failure 'SHA-256 mismatch'
    Assert-True ($script:started.Count -eq $before) 'tampered MSI was executed'
    $script:corrupt = $false
    $script:duplicate = $true
    Expect-Failure 'missing or ambiguous'
    Assert-True ($script:started.Count -eq $before) 'ambiguous checksum MSI was executed'
    $script:duplicate = $false
    $script:failDownload = $true
    Expect-Failure 'synthetic download failed'
    Assert-True ($script:started.Count -eq $before) 'incomplete download was executed'
    $script:failDownload = $false
    $script:tag = 'v9.8.7-beta.1'
    $requestsBefore = $script:requests.Count
    Expect-Failure 'could not resolve a stable release'
    Assert-True ($script:requests.Count -eq ($requestsBefore + 1)) 'invalid release downloaded assets'
    $script:unsupported = $true
    $requestsBefore = $script:requests.Count
    Expect-Failure 'unsupported Windows architecture'
    Assert-True ($script:requests.Count -eq $requestsBefore) 'unsupported architecture accessed network'
    foreach ($download in $script:downloads) {
        Assert-True (-not (Test-Path -LiteralPath (Split-Path -Parent $download))) 'download left temporary files'
    }
} finally {
    Remove-Item -LiteralPath $script:fixture -Recurse -Force
}
Write-Output 'Windows installer tests passed'
