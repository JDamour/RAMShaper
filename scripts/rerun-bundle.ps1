# AT-22 rerun-bundle assembler (RS-024/029 evidence class: "Scripts,
# checksums, raw data and reports").
#
# Assembles everything a second operator needs to reproduce an A/B trial
# analysis into one directory:
#   raw/                  - the frozen preregistration, the analysis report
#                           and any raw data files (copied verbatim)
#   provenance.json       - regenerated build provenance (via
#                           provenance.ps1; the run's toolchain/binaries)
#   SHA256SUMS.txt        - SHA-256 of every bundled file
#   RERUN.md              - the exact rerun command sequence
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\rerun-bundle.ps1 `
#       -Prereg <PATH> -Report <PATH> -Out <DIR> [-Raw <PATH> ...] `
#       [-ReleaseDir <DIR>]
#
# Never overwrites an existing bundle directory: a rerun bundle is
# evidence, and evidence is immutable once assembled.

param(
    [Parameter(Mandatory = $true)][string]$Prereg,
    [Parameter(Mandatory = $true)][string]$Report,
    [Parameter(Mandatory = $true)][string]$Out,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Raw,
    [string]$ReleaseDir = "target\x86_64-pc-windows-msvc\release"
)

$ErrorActionPreference = 'Stop'

if (Test-Path $Out) {
    Write-Error "bundle directory '$Out' already exists; a rerun bundle is immutable once assembled - choose a new directory"
    exit 2
}

foreach ($inputPath in @($Prereg, $Report) + @($Raw)) {
    if ($inputPath -and -not (Test-Path $inputPath)) {
        Write-Error "input file not found: $inputPath"
        exit 2
    }
}

New-Item -ItemType Directory -Path $Out | Out-Null
# Resolve to a full path: the checksum step subtracts this prefix from
# absolute file paths, and the caller may pass a relative -Out.
$Out = (Resolve-Path $Out).ProviderPath
New-Item -ItemType Directory -Path (Join-Path $Out 'raw') | Out-Null

# 1. The inputs, verbatim.
Copy-Item $Prereg (Join-Path $Out 'raw\preregistration.json')
Copy-Item $Report (Join-Path $Out 'raw\analysis-report.json')
foreach ($file in $Raw) {
    Copy-Item $file (Join-Path $Out 'raw')
}

# 2. Provenance, regenerated into the bundle (the run's toolchain and
#    binaries at analysis time). Falls back to a placeholder when the
#    provenance script is unavailable so a remote operator can still
#    bundle; the placeholder says so instead of pretending.
$provScript = Join-Path $PSScriptRoot 'provenance.ps1'
$provOut = Join-Path $Out 'provenance.json'
if (Test-Path $provScript) {
    Push-Location (Get-Location)
    & $provScript -Out $provOut -ReleaseDir $ReleaseDir | Out-Null
    Pop-Location
} else {
    [System.IO.File]::WriteAllText($provOut,
        "{ `"schema`": `"ramshaper.provenance.v1`", `"note`": `"provenance.ps1 not found beside rerun-bundle.ps1; regenerate manually`" }",
        [System.Text.UTF8Encoding]::new($false))
}

# 3. Checksums for every bundled file (relative paths, sorted).
# A statement loop, not ForEach-Object: PS 5.1 threw MethodNotFound on
# the pipeline form here (same script otherwise identical).
$files = Get-ChildItem $Out -Recurse -File | Sort-Object FullName
$sums = foreach ($file in $files) {
    # PS 5.1 (.NET Framework) has no [IO.Path]::GetRelativePath; every
    # bundled file is under $Out, so a substring is the relative path.
    $relative = $file.FullName.Substring($Out.Length + 1).Replace([char]92, '/')
    $hash = (Get-FileHash -Path $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $relative"
}
[System.IO.File]::WriteAllLines((Join-Path $Out 'SHA256SUMS.txt'), $sums,
    [System.Text.UTF8Encoding]::new($false))

# 4. The rerun instructions.
$assembledAt = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
$rerun = @"
# Rerun bundle

Assembled $assembledAt for AT-22-style
independent reproduction (doc 15, AT-22: baseline/treatment, regression,
missing-value and negative outcomes remain reproducible).

## Contents

- ``raw/preregistration.json`` - the frozen trial plan (thresholds and
  schedule frozen *before* collection; doc 08 section 4).
- ``raw/analysis-report.json`` - the original analysis report
  (``ramshaper.ab_analysis.v1``).
- ``raw/`` - any additional raw data files the analyst supplied.
- ``provenance.json`` - the build provenance of the tools that produced
  the report (binaries, toolchain, lockfile, git HEAD).
- ``SHA256SUMS.txt`` - SHA-256 of every file in this bundle. Verify first:
  ``Get-FileHash`` each file and compare.

## Rerun procedure

1. Check out the git HEAD recorded in ``provenance.json`` and rebuild:
   ``cargo build --release --target x86_64-pc-windows-msvc --locked``.
2. Verify the rebuilt binaries' SHA-256 against ``provenance.json``.
   (Reproducible-build caveats apply: path-dependent builds may differ;
   a mismatch means re-verify sources, not ignore.)
3. Re-run the trial per ``raw/preregistration.json`` (the schedule and
   thresholds are frozen; do not re-tune):
   ``ramshaper-bench workload-ab --pilot-blocks <planned> --events
   <planned> --seed <planned> --preregistration raw/preregistration.json``
4. Re-analyze with the frozen bootstrap parameters:
   ``ramshaper-bench analyze-ab --blocks <fresh blocks JSON>
   --preregistration raw/preregistration.json``
5. Compare the fresh report against ``raw/analysis-report.json``: the
   label, the per-pair deltas' signs and the missing-value counts are the
   reproduction targets (latency *values* are machine-dependent; the
   decision structure is not).

## Notes

- A preregistered trial with a ``none``/observe-only treatment honestly
  shows no benefit; that negative outcome is preserved, not tuned away.
- Missing values and excluded pairs are recorded in the report (never
  silently dropped); a rerun must reproduce the same exclusions given the
  same raw data.
"@
[System.IO.File]::WriteAllText((Join-Path $Out 'RERUN.md'), $rerun,
    [System.Text.UTF8Encoding]::new($false))

Write-Output "rerun bundle assembled: $Out"
Write-Output ($sums | Out-String)
