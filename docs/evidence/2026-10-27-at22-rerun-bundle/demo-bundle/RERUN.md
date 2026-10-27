# Rerun bundle

Assembled 2026-10-27T14:33:44Z for AT-22-style
independent reproduction (doc 15, AT-22: baseline/treatment, regression,
missing-value and negative outcomes remain reproducible).

## Contents

- `raw/preregistration.json` - the frozen trial plan (thresholds and
  schedule frozen *before* collection; doc 08 section 4).
- `raw/analysis-report.json` - the original analysis report
  (`ramshaper.ab_analysis.v1`).
- `raw/` - any additional raw data files the analyst supplied.
- `provenance.json` - the build provenance of the tools that produced
  the report (binaries, toolchain, lockfile, git HEAD).
- `SHA256SUMS.txt` - SHA-256 of every file in this bundle. Verify first:
  `Get-FileHash` each file and compare.

## Rerun procedure

1. Check out the git HEAD recorded in `provenance.json` and rebuild:
   `cargo build --release --target x86_64-pc-windows-msvc --locked`.
2. Verify the rebuilt binaries' SHA-256 against `provenance.json`.
   (Reproducible-build caveats apply: path-dependent builds may differ;
   a mismatch means re-verify sources, not ignore.)
3. Re-run the trial per `raw/preregistration.json` (the schedule and
   thresholds are frozen; do not re-tune):
   `ramshaper-bench workload-ab --pilot-blocks <planned> --events
   <planned> --seed <planned> --preregistration raw/preregistration.json`
4. Re-analyze with the frozen bootstrap parameters:
   `ramshaper-bench analyze-ab --blocks <fresh blocks JSON>
   --preregistration raw/preregistration.json`
5. Compare the fresh report against `raw/analysis-report.json`: the
   label, the per-pair deltas' signs and the missing-value counts are the
   reproduction targets (latency *values* are machine-dependent; the
   decision structure is not).

## Notes

- A preregistered trial with a `none`/observe-only treatment honestly
  shows no benefit; that negative outcome is preserved, not tuned away.
- Missing values and excluded pairs are recorded in the report (never
  silently dropped); a rerun must reproduce the same exclusions given the
  same raw data.