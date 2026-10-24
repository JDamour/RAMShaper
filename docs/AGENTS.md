# Instructions for software agents working on RAMShaper

Read `docs/09_SECURITY_PRIVACY_AND_SAFETY.md` before writing code. Read `docs/11_AI_DEVELOPER_BUILD_PROMPT.md` for the phased implementation contract.

1. Never fake test results, builds, OS API support or benchmark numbers.
2. Use documented OS APIs; no kernel drivers or process injection.
3. Observe Only is the default; no silent kills, suspension, working-set trimming, hard memory caps or foreground Realtime priority.
4. Job Objects are not security sandboxes; don't auto-attach arbitrary existing processes; do not set kill-on-job-close.
5. Treat OS call failure and access denied as expected and safe.
6. Journal reversible changes before applying; revalidate PID + creation identity; handle restart.
7. Use the UI PNGs only as visual references, not feature/claim specifications.
8. Keep RAMShaper itself lean and profile agent overhead.
9. Add tests, document measured evidence and update ADR/source references after decisions.
10. When implementation is requested, start with tickets RS-001 through RS-006 only and report actual test commands/results. A documentation task does not authorize implementation.
11. Read `docs/00_IMPLEMENTATION_READINESS.md`, `docs/14_RUNTIME_CONTRACTS.md` and `docs/15_ACCEPTANCE_TEST_MATRIX.md` for scope, behavior and evidence gates. Individual numbered documents are canonical; the original research is historical and the master reference is a reading guide.
12. Use the exact Windows creation FILETIME for process identity; duration clocks are separate. Validate and write through the same process handle. Restore recorded state, preserving external-setting conflicts.
13. A profile/import is not consent. No recurring policies or child-process grants in the first policy alpha. No suspended-create launcher fallback without resolving its crash-safety gate.
