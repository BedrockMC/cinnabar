# Extract installed File-table payloads without invoking installer actions or changing the
# machine. WiX exports cabinets under File/<MSI File id>, so reconstruct the installed names
# and folders from its decompiled authoring before running packaging/check-payload.sh.
param(
    [Parameter(Mandatory = $true)][string]$Msi,
    [Parameter(Mandatory = $true)][string]$Out,
    [string]$Wix = "wix"
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Assert-LeafName([string]$Name, [string]$Kind) {
    if ([string]::IsNullOrEmpty($Name) -or $Name -in @(".", "..") -or
        $Name -match '[<>:"/\\|?*\x00-\x1f]' -or $Name -match '[. ]$' -or
        $Name -match '^(?i:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)') {
        throw "Unsafe $Kind in MSI: '$Name'"
    }
}

$msiPath = (Resolve-Path -LiteralPath $Msi).Path
if (-not (Test-Path -LiteralPath $msiPath -PathType Leaf)) {
    throw "MSI is not a file: $Msi"
}
$outPath = [IO.Path]::GetFullPath($Out)
if (Test-Path -LiteralPath $outPath) {
    throw "Extraction output must not already exist: $outPath"
}
$parent = [IO.Path]::GetDirectoryName($outPath)
New-Item -ItemType Directory -Path $parent -Force | Out-Null
$work = Join-Path $parent (".cinnabar-msi-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $work | Out-Null
try {
    $exports = Join-Path $work "exports"
    $authoring = Join-Path $work "package.wxs"
    $intermediate = Join-Path $work "intermediate"
    $image = Join-Path $work "image"
    # WiX's decompiler extracts embedded cabinets and streams; no msiexec elevation,
    # administrative-install support, product registration or custom action is required.
    $global:LASTEXITCODE = 0
    & $Wix msi decompile $msiPath -x $exports -o $authoring -intermediateFolder $intermediate
    if ($LASTEXITCODE -ne 0) {
        throw "WiX MSI decompilation failed with exit code $LASTEXITCODE"
    }
    $readerSettings = [Xml.XmlReaderSettings]::new()
    $readerSettings.DtdProcessing = [Xml.DtdProcessing]::Prohibit
    $reader = [Xml.XmlReader]::Create($authoring, $readerSettings)
    $document = [Xml.XmlDocument]::new()
    $document.XmlResolver = $null
    try { $document.Load($reader) } finally { $reader.Dispose() }
    $namespace = [Xml.XmlNamespaceManager]::new($document.NameTable)
    $namespace.AddNamespace("w", "http://wixtoolset.org/schemas/v4/wxs")
    $files = $document.SelectNodes("/w:Wix/w:Package//w:File", $namespace)
    if ($files.Count -eq 0) { throw "MSI contains no installed File-table payload" }
    $ids = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $targets = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $plan = [Collections.Generic.List[object]]::new()
    $fileExports = Join-Path $exports "File"
    foreach ($file in $files) {
        $id = $file.GetAttribute("Id")
        $name = $file.GetAttribute("Name")
        Assert-LeafName $id "File id"
        Assert-LeafName $name "file name"
        if (-not $ids.Add($id)) { throw "Duplicate MSI File id: $id" }
        if ($file.ParentNode.LocalName -ne "Component") {
            throw "File $id has no installed Component"
        }
        $parts = [Collections.Generic.List[string]]::new()
        $parts.Add($name)
        $anchored = $false
        for ($node = $file.ParentNode.ParentNode; $null -ne $node; $node = $node.ParentNode) {
            switch ($node.LocalName) {
                "Directory" {
                    if ($node.GetAttribute("Id") -eq "TARGETDIR") { $anchored = $true; continue }
                    $directoryName = $node.GetAttribute("Name")
                    # WiX omits Name when the Directory table's target name is ".".
                    if ([string]::IsNullOrEmpty($directoryName) -or $directoryName -eq ".") { continue }
                    Assert-LeafName $directoryName "directory name"
                    $parts.Insert(0, $directoryName)
                }
                "StandardDirectory" {
                    $standard = $node.GetAttribute("Id")
                    Assert-LeafName $standard "standard directory"
                    if ($standard -ne "TARGETDIR") { $parts.Insert(0, $standard) }
                    $anchored = $true
                }
                "DirectoryRef" { throw "Unresolved DirectoryRef for File $id" }
            }
        }
        if (-not $anchored) { throw "File $id has no installation directory" }
        $relative = [string]::Join([IO.Path]::DirectorySeparatorChar, $parts)
        if (-not $targets.Add($relative)) { throw "Colliding installed MSI path: $relative" }
        $source = Join-Path $fileExports $id
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
            throw "Missing cabinet payload for File $id"
        }
        $plan.Add(@{ Source = $source; Relative = $relative })
    }
    # Fail closed if the cabinet has files absent from the decompiled File table. Every
    # installed file is retained; unexpected payloads remain visible to the strict checker.
    foreach ($entry in Get-ChildItem -LiteralPath $fileExports -Force) {
        if ($entry.PSIsContainer -or ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -or
            -not $ids.Contains($entry.Name)) {
            throw "Unmapped cabinet payload: $($entry.Name)"
        }
    }
    New-Item -ItemType Directory -Path $image | Out-Null
    foreach ($entry in $plan) {
        $destination = Join-Path $image $entry.Relative
        New-Item -ItemType Directory -Path ([IO.Path]::GetDirectoryName($destination)) -Force | Out-Null
        Copy-Item -LiteralPath $entry.Source -Destination $destination
    }
    Move-Item -LiteralPath $image -Destination $outPath
    Write-Output "Extracted $($plan.Count) installed MSI files to $outPath"
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force
}
