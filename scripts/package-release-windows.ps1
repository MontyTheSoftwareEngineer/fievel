$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$binary = Join-Path $repoRoot "target\release\fievel.exe"
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Built fievel binary not found at $binary; run cargo build --release first."
}

$cargoManifest = Get-Content -LiteralPath (Join-Path $repoRoot "Cargo.toml") -Raw
if ($cargoManifest -notmatch '(?ms)^\[package\]\s.*?^version\s*=\s*"([^"]+)"') {
    throw "Could not read package version from $repoRoot\Cargo.toml."
}
$version = $Matches[1]

$rustcOutput = & rustc -vV
if ($LASTEXITCODE -ne 0) {
    throw "Could not determine the Rust target architecture."
}
$hostTriple = $null
foreach ($line in $rustcOutput) {
    if ($line -match '^host:\s*(.+)$') {
        $hostTriple = $Matches[1]
        break
    }
}
if (-not $hostTriple) {
    throw "Could not determine the Rust target architecture."
}
switch -Regex ($hostTriple) {
    "^x86_64-" { $archiveArchitecture = "x64"; break }
    "^aarch64-" { $archiveArchitecture = "arm64"; break }
    "^i686-" { $archiveArchitecture = "x86"; break }
    default { throw "Unsupported Windows target architecture: $hostTriple" }
}

if ($args.Count -gt 0) {
    $archive = [System.IO.Path]::GetFullPath($args[0])
} else {
    $archive = Join-Path $repoRoot "target\release\fievel-$version-windows-$archiveArchitecture.zip"
}

$archiveDirectory = Split-Path -Parent $archive
New-Item -ItemType Directory -Force -Path $archiveDirectory | Out-Null
$staging = Join-Path ([System.IO.Path]::GetTempPath()) ("fievel-release-" + [guid]::NewGuid().ToString("N"))
$archiveTemp = "$archive.tmp.$([guid]::NewGuid().ToString('N')).zip"

try {
    New-Item -ItemType Directory -Path $staging | Out-Null
    Copy-Item -LiteralPath $binary -Destination (Join-Path $staging "fievel.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $staging
    Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE") -Destination $staging

    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $archiveTemp -Force
    Move-Item -LiteralPath $archiveTemp -Destination $archive -Force
    Write-Output "Created $archive"
} finally {
    if (Test-Path -LiteralPath $staging) {
        Remove-Item -LiteralPath $staging -Recurse -Force
    }
    if (Test-Path -LiteralPath $archiveTemp) {
        Remove-Item -LiteralPath $archiveTemp -Force
    }
}
