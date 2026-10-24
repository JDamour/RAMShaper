# Slint license decision memo (R-02 owner gate, ADR-002)

**Date:** 2026-10-24 • **Prepared for:** the R-02 review (owner decision) • **Status:** decision support, not a decision

**Decision needed:** which Slint 1.18.1 licensing mode RAMShaper adopts before G2
can ship a UI binary. The feasibility half of R-02 is **GO** (doc 16, RS-010
spike + addendum); this memo covers the license half. Terms below are verified
against the upstream license text (`LicenseRef-Slint-Royalty-free-2.0.md` in the
slint-ui/slint repository) on 2026-10-24 — doc 10's [S12] legal review should
re-verify against the current text before publishing anything.

## The three options

| | GPL-3.0-only | Slint Royalty-free 2.0 | Commercial |
|---|---|---|---|
| Cost | Free | Free | Paid, per Slint pricing |
| Shipped app must be | GPL-3.0-compatible (whole combined work) | Any proprietary or open terms | Any |
| Source disclosure | **Required** for the combined work on distribution | Not required | Not required |
| Attribution | Standard GPL notices | **Required**: AboutSlint widget in an About screen, or badge on a public download page | Per contract |
| Standalone Slint distribution | Allowed under GPL | **Prohibited** (must be integrated into an app) | Per contract |
| Embedded systems | Allowed | **Prohibited** | Allowed |
| Exposing Slint APIs to users | Allowed | **Prohibited** | Allowed |
| Open-source (OSI) status | OSI-approved | **Not an OSI license** | N/A |

## Fit against RAMShaper's stated intent (doc 10)

Doc 10 recommends Apache-2.0 for the core, a free core tier, optional paid
Pro/Fleet tiers, and a desktop-only product (Windows first). RAMShaper is a
desktop application running on the user's own general-purpose PC — exactly the
Royalty-free 2.0 "Desktop Application" case; it is not embedded and exposes no
Slint APIs.

- **Apache-2.0 core + GPL-3.0 Slint UI:** legal (Apache-2.0 is GPL-3
  compatible); the *shipped combined application* must then be distributed
  under GPL-3.0 with complete corresponding source — including the UI crate.
  Coherent if RAMShaper wants to *be* open source end to end; heaviest ongoing
  obligation; paid Pro/Fleet tiers remain possible (GPL permits charging) but
  every distribution carries the source-disclosure duty.
- **Apache-2.0 core + Royalty-free 2.0 Slint:** lowest friction for the
  stated plan — no source-disclosure duty, paid tiers unaffected, and one
  concrete obligation: attribution via **(a)** an `AboutSlint` widget in an
  About dialog reachable from a top-level menu, or **(b)** a badge on the
  public download page. Neither mechanism exists in the spike yet; (b) is
  trivial (the download page), (a) is a small G2 UI item if preferred.
  Caveat: the project cannot simultaneously *claim* "the app is 100% open
  source" — the UI dependency is under a non-OSI license (RAMShaper's own
  code can still be Apache-2.0; the claim must be worded carefully).
- **Commercial license:** unnecessary under current scope (desktop, no API
  exposure). Would only become relevant if a Fleet/embedded variant or a
  Slint-API-exposing surface were ever planned.

## Points the owner is actually deciding

1. **"Open source" as a public commitment vs license simplicity.** GPL-3.0
   makes the claim unconditional but imposes source disclosure on every
   distribution; Royalty-free 2.0 keeps distribution obligations minimal but
   forfeits the unqualified open-source claim for the binary.
2. **Attribution mechanism** if Royalty-free 2.0: About dialog (in-product,
   one G2 UI item) or download-page badge (no product change).
3. **Doc 10's Apache-2.0 core recommendation is compatible with every
   option** — the core license choice is orthogonal to the Slint choice.

## Suggested next steps (after the owner picks)

1. Record the decision as an ADR (doc 13) superseding ADR-002's open point.
2. Implement the chosen attribution mechanism before any binary distribution.
3. Re-verify terms (Slint bumps its license versions; the spike pins slint
   1.18.1 exactly, so the pin and the license text should be re-checked
   together at the upgrade after G2).

**Sources:** slint-ui/slint `LICENSES/LicenseRef-Slint-Royalty-free-2.0.md`
(license text, fetched 2026-10-24); slint.dev pricing/terms pages; RAMShaper
docs 10 (business plan), 00 (R-02 row), 13 (ADR-002), 16 (RS-010 spike GO).
