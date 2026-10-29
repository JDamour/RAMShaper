# Provenance manifest refresh — 2026-10-29

`provenance.json` regenerated at the current HEAD (161b60f, tree clean)
per ADR-043 (RS-027 slice 1: the generated manifest supersedes
hand-copied hashes). The previous manifest on record was generated at
the 0.3.41-era tree — it predated the `launch` command, `policies
--trace`, and the subject/trial binaries entirely.

## What this manifest stamps

The **distribution-candidate set** (`target\x86_64-pc-windows-msvc\release`,
per ADR-043) — all six binaries, fresh at this commit:

ramshaper-agent.exe, ramshaper-bench.exe, ramshaper-cli.exe,
ramshaper-ui.exe, ramshaper-subject.exe (first stamping),
ramshaper-trial.exe (first stamping)

plus the Cargo.lock SHA-256 and the pinned toolchain. The set was
rebuilt from HEAD immediately before stamping (release build for the
target triple, 19 m 31 s, clean) — so the hashes certify the current
source, not a leftover. The plain-`target\release` binaries (the soak/
evidence run set) are byte-different (different profile path) and were
verified current in the same session.

## Script gotcha recorded

`scripts/provenance.ps1` resolves its enumeration root relative to the
**caller's working directory**: running it from a subdirectory "succeeds"
but writes a manifest with an EMPTY binaries array and an empty lockfile
entry (it looks for `target\...` under the cwd). Run it from the repo
root (or pass an absolute `-Out`); this evidence dir's first attempt was
exactly that trap.
