---
name: Feature or ticket proposal
about: Propose new work, or pick up a backlog ticket
title: ''
labels: enhancement
assignees: ''
---

<!-- Work is organized by ticket IDs from
     docs/docs/07_MVP_ROADMAP_AND_BACKLOG.md. Proposals that duplicate an
     existing ticket will be linked to it. Check that document and the
     head of docs/CHANGELOG.md before filing. -->

**Ticket ID (if this maps to one):**

RS-XXX from doc 07, or "new" with a one-line summary.

**Which milestone / gate does this belong to?**

M0 CLI · M1 observation · M2 native UI · M3 policy alpha · M4 managed
launch · M5 evidence/distribution (see doc 07's table).

**What should the change do — and what must it never do?**

State the user-visible behavior and the safety boundary explicitly. If
it touches process settings, consent, the journal or recovery, cite the
section of [doc 14](docs/docs/14_RUNTIME_CONTRACTS.md) that governs it.

**What evidence would demonstrate completion?**

Per doc 15's bar: tests (run on Windows as a standard user), a doc 16
record, and any live measurement the claim depends on. "It works on my
machine" is not evidence; a recorded run is.

**Dependencies / prerequisites**

Which tickets or owner decisions (licensing, cohorts, packaging) must
land first?
