# RS-027 provenance manifest generator.
#
# Emits a JSON manifest of the built release artifacts: toolchain versions,
# per-binary path/size/SHA-256, the committed lockfile hash and the git
# HEAD when available. This automates the artifact identification that the
# evidence records previously made by hand (doc 15's evidence template:
# "Artifacts and SHA-256").
#
# Usage:  powershell -NoProfile -ExecutionPolicy Bypass -File scripts\provenance.ps1
# Output: provenance.json next to the script's caller (or -Out <path>).
# Read-only: hashes files, never modifies them.

param(
    [string]$Out = "provenance.json",
    [string]$ReleaseDir = "target\x86_64-pc-windows-msvc\release"
)

$ErrorActionPreference = 'Stop'

function Sha256([string]$Path) {
    (Get-FileHash -Path $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

$manifest = [ordered]@{
    schema          = 'ramshaper.provenance.v1'
    generated_at_utc = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
    toolchain       = [ordered]@{}
    lockfile        = [ordered]@{}
    binaries        = @()
    git             = [ordered]@{}
}

# Toolchain versions (best-effort; a missing tool is recorded as absent).
foreach ($tool in @('rustc', 'cargo')) {
    $version = & $tool --version 2>$null | Select-Object -First 1
    $manifest.toolchain[$tool] = if ($version) { $version.Trim() } else { 'absent' }
}

# Lockfile identity: hash + the pinned toolchain file.
if (Test-Path 'Cargo.lock') {
    $manifest.lockfile.cargo_lock_sha256 = Sha256 'Cargo.lock'
}
if (Test-Path 'rust-toolchain.toml') {
    $manifest.lockfile.rust_toolchain = (Get-Content 'rust-toolchain.toml' -Raw).Trim()
}

# Release binaries.
if (Test-Path $ReleaseDir) {
    foreach ($exe in Get-ChildItem $ReleaseDir -Filter 'ramshaper-*.exe' | Sort-Object Name) {
        $manifest.binaries += [ordered]@{
            name      = $exe.Name
            path      = $exe.FullName
            size_bytes = $exe.Length
            sha256    = Sha256 $exe.FullName
        }
    }
}

# Git HEAD when available (the workspace may be built from a tree without git).
$head = & git rev-parse HEAD 2>$null
$manifest.git.head = if ($head) { $head.Trim() } else { 'unavailable' }
$dirty = & git status --porcelain 2>$null
$manifest.git.working_tree = if ($null -ne $dirty -and $dirty.Count -gt 0) { 'dirty' } else { 'clean' }

$json = $manifest | ConvertTo-Json -Depth 5
# BOM-less UTF-8: Set-Content's -Encoding UTF8 writes a BOM in PS 5.1,
# which breaks JSON consumers.
# An absolute -Out (e.g. passed by rerun-bundle.ps1) must not be joined
# onto the current directory.
$OutPath = if ([System.IO.Path]::IsPathRooted($Out)) { $Out } else { Join-Path (Get-Location) $Out }
[System.IO.File]::WriteAllText($OutPath, $json + [Environment]::NewLine, [System.Text.UTF8Encoding]::new($false))
Write-Output "provenance manifest written: $Out"
Write-Output $json
