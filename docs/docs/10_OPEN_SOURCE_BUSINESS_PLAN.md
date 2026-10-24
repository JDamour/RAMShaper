# RAMShaper — Open-Source Strategy, Positioning and Business Hypotheses

**Status:** business hypotheses only. No revenue estimates or market demand claims are validated.

## Vision

Help extend the useful life of constrained desktop computers, with users in control of every optimization. Publish verifiable test methodology and avoid the trust problems common to “PC optimizer” marketing.

## Positioning

**Product category:** Resource-aware desktop workload manager.  
**One-line pitch:** “RAMShaper detects real contention, safely manages approved background work, and measures whether your PC became more responsive.”

**Trust propositions:** local-first, minimal overhead, observable actions, rollback-first, reproducible benchmark reports, honest “no change” reporting.

## Audience sequencing

1. Technical early adopters who can reproduce and report results; Windows 11 users.
2. Owners of 4–8GB PCs with browser-heavy workflows and pain from app switching.
3. Refurbishers and community organizations interested in low-cost computer reuse.
4. School/computer lab IT admins and small business fleets (only after policy stability).

Avoid promising RAMShaper fixes underpowered hardware. Realistic hardware upgrades or removing bloat may be more appropriate.

## Open-source licensing options (decision needed)

- **Apache-2.0 or MIT:** easy adoption and contributions, including commercial use; competitors can fork.
- **GPLv3:** stronger copyleft obligations; must check compatibility with selected Slint licensing/distribution mode.
- **Dual-licensing/enterprise:** commercial complexity and legal review; do not assume changing an established license later is easy, particularly with outside contributors.
- **Open core:** core engine and basic GUI free, fleet policy distribution/support optional paid offering; transparency must remain.

**Recommended for initial legal review:** Apache-2.0 for core with clear dependency/license compliance plan; however **do not finalize or publish a license** until checking Slint's current GPLv3 vs royalty-free desktop licensing and attribution requirements [S12]. Contributions require a documented license and contributor policies.

## Potential packages (hypotheses)

| Tier | Features | Possible monetization |
|---|---|---|
| Free/Core | Pressure monitoring, safe app groups, manual policies, benchmark exports | Community, GitHub Sponsors, donations |
| Pro | Advanced local profiles, scheduled policy workflows, richer evidence | Optional one-time or subscription, test user willingness |
| Fleet | Signed policies, managed rollouts, host diagnostics and compliance-safe reporting | Per-device licensing/support after admin pilot |
| OEM/refurbisher | Deployment customization, reproducible fleet benchmarking | Services or licensing agreement |

Do not restrict essential safety fixes or rollback to paid features. Avoid recurring account dependency for offline personal use.

## Research validation plan

- 20–30 qualitative interviews with old-PC owners, 5–10 IT admins; record current tools and frustrations.
- Prototype observe-only UI and ask for “would keep installed” after one week of use.
- Measure actual retained weekly active use and uninstall rate (only where users agree to analytics; prefer voluntary surveys).
- Run device-lab pilot: collect *consented, anonymized* before/after benchmarks.
- Test two pricing alternatives via landing-page surveys/pilot contracts before choosing recurring price.

## Differentiation research and red lines

Competitor Process Lasso has a mature CPU scheduling product and related memory prioritization [S15][S16]. Microsoft PC Manager already covers cleanup and startup features [S17]. Do not copy their marketing claims or imply nonexistent first-mover advantage. RAMShaper's possible defensibility: open benchmark corpus, low-overhead implementation, safe per-app managed launch, understandable UX, hardware-cohort specialization.

## Launch sequence

1. Public repo and problem/measurement statement.
2. Read-only beta and an honest benchmark harness.
3. Community feedback and compatibility collection.
4. Opt-in safe policies with action logs and rollback.
5. Publish positive, negative and neutral results.
6. Open-source maintainership, security disclosure and contributor guide.
7. Explore paid fleet features only once personal/core utility is stable.

## Brand and legal considerations

“RAMShaper” is a **working product name**, not a registered trademark or verified available domain. Perform independent trademark/domain search before launch. Branding should say “for Windows” or “Windows-compatible” rather than implying Microsoft affiliation. Generated design images are references, not executable UI or validated data.

## Landing-page draft

> **RAMShaper**  
> Keep your computer responsive under pressure.  
> RAMShaper is a lightweight, local-first Rust tool that monitors memory contention, helps manage selected background apps, and shows transparent performance measurements. No fake RAM boosts. No mystery cleanup. You stay in control.

## Potential environmental impact

Extending the use of aging PCs could reduce premature replacement **if** the tool demonstrably improves usability and longevity. Do not quantify avoided e-waste or carbon savings without lifecycle data.
