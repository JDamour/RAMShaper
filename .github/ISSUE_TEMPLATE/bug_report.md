---
name: Bug report
about: Something crashed, misbehaved, or printed an error you didn't expect
title: ''
labels: bug
assignees: ''
---

<!-- Before filing: check git log + docs/CHANGELOG.md head — the defect may
     already be recorded in docs/docs/16_IMPLEMENTATION_EVIDENCE.md. -->

**What were you trying to do?**

The exact command(s), e.g. `ramshaper-cli low-memory-watch --interval-ms 500`,
or `ramshaper-agent serve --policy-journal <PATH>`.

**What happened?**

The full raw output (stdout **and** stderr), unmodified. The tools print
typed error codes (`access_denied`, `exited_87`, `win32_<code>`,
`Unsupported: ...`) — that is deliberate, please don't paraphrase them.

**What did you expect instead?**

**Environment**

- Windows build (`winver` → e.g. 26200.9711):
- RAMShaper build: commit hash (or the documentation revision from
  `README.md` if built from an archive):
- Was a policy-journal agent running? (yes/no; if yes, was `ENABLE`
  typed on that console?)

**Evidence**

- [ ] Raw output attached (redact process listings from sensitive machines)
- [ ] Steps to reproduce, minimal if possible
- [ ] Screenshots only if the UI is involved (the Slint window)

**Anything else?**

If you suspect a safety-relevant defect (an action that was not restored,
a journal that would not reconcile, a consent bypass), say so at the top
— those are triaged first and get a doc 16 defect record even when
unreproducible.
