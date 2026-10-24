# Slint license options for ramshaper-ui (G2 decision note)

**Note (2026-10-24, later):** a parallel session's verified memo now exists at
[2026-10-24-slint-license-memo.md](2026-10-24-slint-license-memo.md) (linked
from ADR-002 in doc 13, checked against the upstream RF-2.0 license text —
including that v2.0 has no selling prohibition and the license is not OSI).
Treat that memo as primary; this note remains for the doc-10 coherence
analysis below.

**Status: decision needed (owner).** This note stages the facts for the G2 gate
requirement "Slint license/accessibility decision". It is not a decision record;
the pick gets recorded as an ADR in doc 13 once made.

## The constraint that decides it

Doc 10 (open-source business plan) recommends **Apache-2.0 for the core** as the
initial direction. Slint's GPLv3 option requires the linked application to be
GPL-compatible — adopting it would force RAMShaper to GPLv3 and override doc 10's
recommendation. The royalty-free desktop license imposes no copyleft: your own
source can stay Apache-2.0/MIT.

## Options

| | Slint GPLv3 | Slint royalty-free desktop | Slint commercial |
|---|---|---|---|
| Cost | $0 | $0 | paid (pricing at slint.dev) |
| Your code | must become GPL-compatible | any license incl. Apache-2.0 | any |
| Requirements | app released GPLv3 | Slint attribution in the app | none beyond contract |
| Embedded use | allowed | **excluded** | allowed |

The royalty-free attribution requirement (per Slint's FAQ): display the
`AboutSlint` widget in an About screen "accessible from the top level menu"
(or splash screen if none), or show the Slint badge on the public download page.
Other restrictions: no redistributing Slint standalone, no exposing Slint's APIs
through your app, keep Slint source notices intact. RAMShaper is desktop-only
(the embedded exclusion never bites) and its UI crate wraps Slint internally
(no API exposure).

## Implications for RAMShaper

- **Royalty-free + Apache-2.0 core** keeps doc 10's recommended path intact. Cost:
  an `AboutSlint` widget in the planned Safety & Settings screen (doc 06 S07) —
  small UI work, and it doubles as the third-party-notices surface the project
  needs anyway.
- **GPLv3** is simpler (no attribution work) but locks the whole product to GPLv3,
  forecloses the open-core/enterprise packaging hypotheses in doc 10 (per-device
  fleet licensing, OEM deals), and contradicts the doc 10 recommendation.
- **Commercial** is unnecessary for the MVP: no embedded target, attribution is
  acceptable.

## Recommendation

**Slint royalty-free desktop license, app under Apache-2.0**, `AboutSlint` widget
in Safety & Settings, Slint + dependency license notices gathered into the
third-party-notices surface before G2 sign-off. If the owner prefers a strong
copyleft posture for the whole product instead, GPLv3 is the honest alternative —
but it should be a product-strategy choice, not a default drift.

## Sources

- Slint pricing: https://slint.dev/pricing
- License texts: https://github.com/slint-ui/slint/blob/master/LICENSE.md
- FAQ (attribution specifics): https://github.com/slint-ui/slint/blob/master/FAQ.md
- Royalty-free introduction: https://slint.dev/blog/slint-1.1-released
