//! A/B analysis over paired synthetic-workload runs (RS-024 Stage B
//! analysis half; AT-20; doc 08 §4).
//!
//! Pure fixture calculations — **no claim of real benefit**: the numbers
//! this module produces are only as good as the trials fed to it, and the
//! A/B *runner* (which needs the enabled actuator) is a separate, gated
//! piece. Everything here implements doc 08 §4 literally:
//!
//! - per-run p50/p95 by **nearest rank, ceil(q × n), one-based** — and the
//!   cohort p95 is never the average of run percentiles;
//! - sample counts (valid n, missing count) with every missing value's
//!   reason **preserved** — never silently dropped, never imputed;
//! - A/B runs paired **by block**; the paired per-block improvement is
//!   `100 × (A − B) / A`, defined only when `A > 0` (a missing or
//!   nonpositive denominator yields an unavailable delta, never infinity);
//! - a **95% bootstrap interval over whole paired blocks** (10,000
//!   resamples, recorded seed, deterministic PRNG) — never resampling
//!   individual events as if independent; small samples are labelled
//!   exploratory;
//! - the four outcome labels with the preregistered 5% primary threshold,
//!   the 2% / 10% completion guardrails, and the safety-failure override
//!   (a safety failure is a regression regardless of speed).

use serde::Serialize;

/// Report schema version.
pub const ANALYSIS_SCHEMA: &str = "ramshaper.ab_analysis.v1";

/// Bootstrap resamples (doc 08 §4: "10,000 resamples, recorded seed").
pub const BOOTSTRAP_RESAMPLES: usize = 10_000;

/// Preregistered primary threshold: at least 5% lower paired p95 latency.
pub const PRIMARY_BENEFIT_PCT: f64 = 5.0;
/// Foreground completion guardrail: upper interval bound for slowdown no
/// greater than 2%.
pub const FOREGROUND_GUARDRAIL_PCT: f64 = 2.0;
/// Background completion guardrail: upper bound no greater than 10%.
pub const BACKGROUND_GUARDRAIL_PCT: f64 = 10.0;
/// Blocks below this count make the interval **exploratory** (doc 08 §4).
pub const MIN_BLOCKS_FOR_CONFIRMATORY: usize = 5;

/// One measured or missing event of a run.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(untagged)]
pub enum EventValue {
    /// A measured task latency, milliseconds.
    Measured(f64),
    /// A missing value with its preserved reason (never silently dropped).
    Missing {
        /// Why the event is missing (e.g. "app not responding").
        reason: String,
    },
}

/// One run's event series.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct RunEvents {
    /// Run identifier (echoed in the report).
    pub run_id: String,
    /// Event series; missing values carry reasons.
    pub events: Vec<EventValue>,
}

/// One paired A/B block: a baseline and a treatment run under the same
/// preregistered conditions.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Block {
    /// Block identifier (pairing key).
    pub block_id: u32,
    /// The A run.
    pub baseline: RunEvents,
    /// The B run.
    pub treatment: RunEvents,
}

/// Per-run statistics.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunStats {
    /// Echoed run id.
    pub run_id: String,
    /// Valid (measured) event count.
    pub n_valid: usize,
    /// Missing event count (reasons preserved in `missing`).
    pub n_missing: usize,
    /// Preserved missing reasons, in order.
    pub missing: Vec<String>,
    /// Nearest-rank p50, milliseconds (`None` when no valid events).
    pub p50_ms: Option<f64>,
    /// Nearest-rank p95, milliseconds (`None` when no valid events).
    pub p95_ms: Option<f64>,
}

/// Nearest-rank quantile: sort the samples ascending and take position
/// `ceil(q × n)` (one-based). Returns `None` for empty input.
#[must_use]
pub fn nearest_rank(samples: &[f64], q: f64) -> Option<f64> {
    if samples.is_empty() || !(0.0..=1.0).contains(&q) {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    let position = (q * n as f64).ceil();
    let index = position.clamp(1.0, n as f64) as usize - 1;
    sorted.get(index).copied()
}

/// Per-run statistics for one run.
#[must_use]
pub fn run_stats(run: &RunEvents) -> RunStats {
    let mut measured = Vec::new();
    let mut missing = Vec::new();
    for event in &run.events {
        match event {
            EventValue::Measured(value) => measured.push(*value),
            EventValue::Missing { reason } => missing.push(reason.clone()),
        }
    }
    RunStats {
        run_id: run.run_id.clone(),
        n_valid: measured.len(),
        n_missing: missing.len(),
        missing,
        p50_ms: nearest_rank(&measured, 0.50),
        p95_ms: nearest_rank(&measured, 0.95),
    }
}

/// One paired block's primary-endpoint delta.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairDelta {
    /// Block id.
    pub block_id: u32,
    /// Baseline run p95, ms.
    pub baseline_p95_ms: Option<f64>,
    /// Treatment run p95, ms.
    pub treatment_p95_ms: Option<f64>,
    /// Per-pair improvement `100 × (A − B) / A` — defined only when the
    /// baseline p95 exists and is positive; positive means improvement.
    pub improvement_pct: Option<f64>,
}

/// The paired deltas over all blocks (primary endpoint: per-run p95).
#[must_use]
pub fn paired_deltas(blocks: &[Block]) -> Vec<PairDelta> {
    blocks
        .iter()
        .map(|block| {
            let baseline = run_stats(&block.baseline);
            let treatment = run_stats(&block.treatment);
            let improvement_pct = match (baseline.p95_ms, treatment.p95_ms) {
                (Some(a), Some(b)) if a > 0.0 => Some(100.0 * (a - b) / a),
                _ => None, // missing or nonpositive denominator: unavailable
            };
            PairDelta {
                block_id: block.block_id,
                baseline_p95_ms: baseline.p95_ms,
                treatment_p95_ms: treatment.p95_ms,
                improvement_pct,
            }
        })
        .collect()
}

/// Deterministic SplitMix64 — the recorded seed makes every bootstrap
/// interval exactly reproducible (no platform float drift in the *draws*).
#[derive(Debug, Clone)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// A generator from a recorded seed.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Next raw 64-bit value.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform index in `0..bound` (rejection-sampled, unbiased).
    fn below(&mut self, bound: usize) -> usize {
        debug_assert!(bound > 0);
        let bound = bound as u64;
        loop {
            let draw = self.next_u64();
            // Rejection sampling avoids modulo bias.
            let limit = u64::MAX - (u64::MAX % bound);
            if draw < limit {
                return (draw % bound) as usize;
            }
        }
    }
}

/// Bootstrap configuration (defaults per doc 08 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapConfig {
    /// Resample count (default 10,000).
    pub resamples: usize,
    /// Recorded seed (deterministic replays).
    pub seed: u64,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            resamples: BOOTSTRAP_RESAMPLES,
            seed: 0x5EED_5EED_5EED_5EED,
        }
    }
}

/// The paired-block summary: median paired improvement and its 95%
/// bootstrap interval over whole blocks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PairedSummary {
    /// Blocks paired (defined deltas only).
    pub pairs_used: usize,
    /// Blocks excluded from pairing (unavailable deltas), with ids.
    pub pairs_excluded: Vec<u32>,
    /// Median paired improvement, percent (positive = improvement).
    pub median_improvement_pct: Option<f64>,
    /// 95% bootstrap interval, percent (lower, upper).
    pub ci95_improvement_pct: Option<(f64, f64)>,
    /// Resamples used.
    pub resamples: usize,
    /// The recorded seed.
    pub seed: u64,
    /// True when `pairs_used < MIN_BLOCKS_FOR_CONFIRMATORY` (doc 08: label
    /// small-sample intervals exploratory).
    pub exploratory: bool,
}

/// Median of a sample (average of the two middles for even n).
#[must_use]
pub fn median(samples: &[f64]) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len();
    Some(if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    })
}

/// Percentile by nearest rank over a sample (ceil, one-based).
#[must_use]
fn percentile_nearest_rank(mut sorted: Vec<f64>, q: f64) -> Option<f64> {
    nearest_rank(&sorted, q).map(|_| {
        sorted.sort_by(|a, b| a.total_cmp(b));
        let n = sorted.len();
        let index = (q * n as f64).ceil().clamp(1.0, n as f64) as usize - 1;
        sorted[index]
    })
}

/// Bootstrap the paired-block summary from the deltas.
#[must_use]
pub fn bootstrap_summary(deltas: &[PairDelta], config: &BootstrapConfig) -> PairedSummary {
    let defined: Vec<(u32, f64)> = deltas
        .iter()
        .filter_map(|delta| delta.improvement_pct.map(|value| (delta.block_id, value)))
        .collect();
    let excluded = deltas
        .iter()
        .filter(|delta| delta.improvement_pct.is_none())
        .map(|delta| delta.block_id)
        .collect();
    let values: Vec<f64> = defined.iter().map(|(_, value)| *value).collect();
    let median_improvement_pct = median(&values);
    let mut rng = SplitMix64::new(config.seed);
    let ci95_improvement_pct = if defined.is_empty() {
        None
    } else {
        let mut resample_medians = Vec::with_capacity(config.resamples);
        for _ in 0..config.resamples {
            let mut draw: Vec<f64> = Vec::with_capacity(values.len());
            for _ in 0..values.len() {
                let index = rng.below(values.len());
                draw.push(values[index]);
            }
            // Median of the resample (nearest-rank 50th over the draw).
            if let Some(m) = percentile_nearest_rank(draw, 0.50) {
                resample_medians.push(m);
            }
        }
        let lower = percentile_nearest_rank(resample_medians.clone(), 0.025);
        let upper = percentile_nearest_rank(resample_medians, 0.975);
        match (lower, upper) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => None,
        }
    };
    PairedSummary {
        pairs_used: defined.len(),
        pairs_excluded: excluded,
        median_improvement_pct,
        ci95_improvement_pct,
        resamples: config.resamples,
        seed: config.seed,
        exploratory: defined.len() < MIN_BLOCKS_FOR_CONFIRMATORY,
    }
}

/// Guardrail inputs for the verdict (doc 08 §4 planning thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Guardrails {
    /// Foreground completion: upper interval bound for percentage slowdown
    /// no greater than this.
    pub foreground_max_slowdown_pct: f64,
    /// Background completion: upper bound no greater than this.
    pub background_max_slowdown_pct: f64,
}

impl Default for Guardrails {
    fn default() -> Self {
        Self {
            foreground_max_slowdown_pct: FOREGROUND_GUARDRAIL_PCT,
            background_max_slowdown_pct: BACKGROUND_GUARDRAIL_PCT,
        }
    }
}

/// Safety report: any unexpected kill, forced suspension, data corruption
/// or reproducible instability is a safety failure regardless of speed
/// (doc 08 §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SafetyReport {
    /// Whether any safety failure was observed.
    pub safety_failure: bool,
    /// The recorded reason(s), when `safety_failure`.
    pub reasons: Vec<String>,
}

/// The four outcome labels (doc 08 §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OutcomeLabel {
    /// Primary threshold/interval and all guardrails pass.
    ValidatedImprovement,
    /// Underpowered/insufficient/invalid, or uncertainty cannot resolve.
    Inconclusive,
    /// Adequate protocol but the meaningful-gain criterion is not met,
    /// without an established regression.
    NoMeasuredImprovement,
    /// Established regression, guardrail violation or safety failure.
    Regression,
}

impl OutcomeLabel {
    /// Stable machine code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ValidatedImprovement => "validated_improvement",
            Self::Inconclusive => "inconclusive",
            Self::NoMeasuredImprovement => "no_measured_improvement",
            Self::Regression => "regression",
        }
    }
}

/// The full A/B analysis report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AbReport {
    /// Report schema.
    pub schema: &'static str,
    /// Per-run statistics (every run, including excluded ones).
    pub runs: Vec<RunStats>,
    /// Paired deltas per block.
    pub pairs: Vec<PairDelta>,
    /// The paired summary (median + bootstrap interval).
    pub summary: PairedSummary,
    /// The outcome label.
    pub label: OutcomeLabel,
    /// Why: the label rule's evaluation trail.
    pub reasons: Vec<String>,
    /// Absolute p95 values are carried in `runs`/`pairs` beside every
    /// percentage (doc 08: report absolute values beside percentages).
    pub guardrails: Guardrails,
    pub safety: SafetyReport,
}

/// Analyze paired blocks into the full report.
#[must_use]
pub fn analyze(
    blocks: &[Block],
    config: &BootstrapConfig,
    guardrails: &Guardrails,
    safety: &SafetyReport,
) -> AbReport {
    let runs: Vec<RunStats> = blocks
        .iter()
        .flat_map(|block| [run_stats(&block.baseline), run_stats(&block.treatment)])
        .collect();
    let deltas = paired_deltas(blocks);
    let summary = bootstrap_summary(&deltas, config);

    let mut reasons = Vec::new();
    let stats_pair = (summary.median_improvement_pct, summary.ci95_improvement_pct);
    let label = if safety.safety_failure {
        reasons.push("safety failure overrides any speed result".to_string());
        OutcomeLabel::Regression
    } else if summary.pairs_used == 0 {
        reasons.push("no pairable blocks: nothing to compare".to_string());
        OutcomeLabel::Inconclusive
    } else if let (Some(median), Some((lo, hi))) = stats_pair {
        if summary.exploratory {
            reasons.push(format!(
                "exploratory: {} pairable blocks < {MIN_BLOCKS_FOR_CONFIRMATORY}",
                summary.pairs_used
            ));
        }
        // Guardrail: a slowdown means negative improvement; the interval's
        // LOWER bound is the worst-case improvement. A foreground slowdown
        // guardrail passes when the interval's lower bound (worst case)
        // is not a slowdown beyond the limit, i.e. `-lo <= limit`.
        let slowdown_beyond = |limit: f64| -> bool { -lo > limit };
        let foreground_violated = slowdown_beyond(guardrails.foreground_max_slowdown_pct);
        let background_violated = slowdown_beyond(guardrails.background_max_slowdown_pct);
        if foreground_violated || background_violated {
            reasons.push(format!(
                "completion guardrail violated: worst-case improvement {lo:.2}% implies a slowdown beyond the bound"
            ));
            OutcomeLabel::Regression
        } else if median >= PRIMARY_BENEFIT_PCT && lo > 0.0 && !summary.exploratory {
            reasons.push(format!(
                "median improvement {median:.2}% >= {PRIMARY_BENEFIT_PCT}% and interval [{lo:.2}, {hi:.2}] excludes zero"
            ));
            OutcomeLabel::ValidatedImprovement
        } else if median < 0.0 && hi < 0.0 {
            // The whole interval is on the regression side: established.
            reasons.push(format!(
                "established regression: median {median:.2}%, interval [{lo:.2}, {hi:.2}] entirely below zero"
            ));
            OutcomeLabel::Regression
        } else if summary.exploratory {
            OutcomeLabel::Inconclusive
        } else {
            reasons.push(format!(
                "adequate protocol; meaningful gain criterion not met (median {median:.2}%, interval [{lo:.2}, {hi:.2}])"
            ));
            OutcomeLabel::NoMeasuredImprovement
        }
    } else {
        reasons.push("summary statistics unavailable".to_string());
        OutcomeLabel::Inconclusive
    };

    AbReport {
        schema: ANALYSIS_SCHEMA,
        runs,
        pairs: deltas,
        summary,
        label,
        reasons,
        guardrails: *guardrails,
        safety: safety.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured(values: &[f64]) -> Vec<EventValue> {
        values.iter().map(|v| EventValue::Measured(*v)).collect()
    }

    /// AT-20's core: the fixture math matches independently calculated
    /// values. Baseline events 10..=19 ms (10 events): p95 = ceil(0.95*10)
    /// = position 10 (one-based) = 19.0; p50 = position 5 = 14.0? No —
    /// sorted [10..19], position ceil(0.5*10)=5 → 14.0.
    #[test]
    fn nearest_rank_ceil_matches_hand_computation() {
        let samples: Vec<f64> = (10..=19).map(f64::from).collect();
        assert_eq!(nearest_rank(&samples, 0.95), Some(19.0));
        assert_eq!(nearest_rank(&samples, 0.50), Some(14.0));
        // Single sample: q=0.95 → ceil(0.95)=1.
        assert_eq!(nearest_rank(&[42.0], 0.95), Some(42.0));
        // Empty and out-of-range q.
        assert_eq!(nearest_rank(&[], 0.95), None);
        assert_eq!(nearest_rank(&samples, 1.5), None);
    }

    #[test]
    fn missing_values_are_preserved_and_counted_never_imputed() {
        let run = RunEvents {
            run_id: "r1".into(),
            events: vec![
                EventValue::Measured(10.0),
                EventValue::Missing {
                    reason: "event timeout".into(),
                },
                EventValue::Measured(30.0),
                EventValue::Missing {
                    reason: "app killed".into(),
                },
            ],
        };
        let stats = run_stats(&run);
        assert_eq!(stats.n_valid, 2);
        assert_eq!(stats.n_missing, 2);
        assert_eq!(stats.missing, vec!["event timeout", "app killed"]);
        // p95 over the 2 valid events: ceil(0.95*2)=2 → 30.0. Missing
        // values did not shrink the rank or get imputed.
        assert_eq!(stats.p95_ms, Some(30.0));
    }

    /// Hand-computed paired fixture: baseline p95s [100, 200, 50, 80],
    /// treatment p95s [90, 210, 45, 80].
    /// Improvements: 100*(100-90)/100 = 10%; 100*(200-210)/200 = -5%;
    /// 100*(50-45)/50 = 10%; 100*(80-80)/80 = 0%.
    fn fixture_blocks() -> Vec<Block> {
        let block = |id: u32, a: &[f64], b: &[f64]| Block {
            block_id: id,
            baseline: RunEvents {
                run_id: format!("a-{id}"),
                events: measured(a),
            },
            treatment: RunEvents {
                run_id: format!("b-{id}"),
                events: measured(b),
            },
        };
        vec![
            block(1, &[100.0; 20][..], &[90.0; 20][..]), // p95 100 vs 90 → +10%
            block(2, &[200.0; 20][..], &[210.0; 20][..]), // p95 200 vs 210 → −5%
            block(3, &[50.0; 20][..], &[45.0; 20][..]),  // p95 50 vs 45 → +10%
            block(4, &[80.0; 20][..], &[80.0; 20][..]),  // p95 80 vs 80 → 0%
        ]
    }

    #[test]
    fn paired_deltas_match_independently_calculated_percentages() {
        let deltas = paired_deltas(&fixture_blocks());
        let expected = [10.0, -5.0, 10.0, 0.0];
        for (index, (delta, expected)) in deltas.iter().zip(expected).enumerate() {
            assert_eq!(delta.block_id, index as u32 + 1);
            let improvement = delta.improvement_pct.expect("defined");
            assert!(
                (improvement - expected).abs() < 1e-9,
                "pair {}: {improvement} vs {expected}",
                delta.block_id
            );
        }
    }

    #[test]
    fn nonpositive_denominator_yields_unavailable_never_infinity() {
        let blocks = vec![Block {
            block_id: 1,
            baseline: RunEvents {
                run_id: "a".into(),
                events: vec![EventValue::Missing {
                    reason: "no events".into(),
                }],
            },
            treatment: RunEvents {
                run_id: "b".into(),
                events: measured(&[90.0]),
            },
        }];
        let deltas = paired_deltas(&blocks);
        assert_eq!(deltas[0].improvement_pct, None);
        let summary = bootstrap_summary(&deltas, &BootstrapConfig::default());
        assert_eq!(summary.pairs_used, 0);
        assert_eq!(summary.pairs_excluded, vec![1]);
        assert!(summary.exploratory);
    }

    #[test]
    fn bootstrap_is_deterministic_for_a_recorded_seed() {
        let deltas = paired_deltas(&fixture_blocks());
        let config = BootstrapConfig {
            resamples: 1_000,
            seed: 42,
        };
        let first = bootstrap_summary(&deltas, &config);
        let second = bootstrap_summary(&deltas, &config);
        assert_eq!(first, second);
        // A different seed yields a different (but same-shaped) interval.
        let other = bootstrap_summary(
            &deltas,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 43,
            },
        );
        assert_eq!(first.pairs_used, other.pairs_used);
    }

    #[test]
    fn labels_follow_the_doc08_decision_table() {
        let guardrails = Guardrails::default();
        let no_safety = SafetyReport {
            safety_failure: false,
            reasons: vec![],
        };
        let safety_failure = SafetyReport {
            safety_failure: true,
            reasons: vec!["treatment app killed".into()],
        };

        // Strong benefit across many blocks: validated (5 blocks of +10%).
        let good = paired_deltas(&{
            (1..=5)
                .map(|id| Block {
                    block_id: id,
                    baseline: RunEvents {
                        run_id: format!("a-{id}"),
                        events: measured(&[100.0; 20]),
                    },
                    treatment: RunEvents {
                        run_id: format!("b-{id}"),
                        events: measured(&[90.0; 20]),
                    },
                })
                .collect::<Vec<_>>()
        });
        let report = analyze(
            &(1..=5)
                .map(|id| Block {
                    block_id: id,
                    baseline: RunEvents {
                        run_id: format!("a-{id}"),
                        events: measured(&[100.0; 20]),
                    },
                    treatment: RunEvents {
                        run_id: format!("b-{id}"),
                        events: measured(&[90.0; 20]),
                    },
                })
                .collect::<Vec<_>>(),
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &no_safety,
        );
        assert_eq!(report.label, OutcomeLabel::ValidatedImprovement);
        assert!(!report.summary.exploratory);
        let _ = good;

        // A safety failure is a regression regardless of speed.
        let report = analyze(
            &fixture_blocks(),
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &safety_failure,
        );
        assert_eq!(report.label, OutcomeLabel::Regression);
        assert!(report.reasons[0].contains("safety failure"));

        // Few blocks: exploratory → Inconclusive when nothing else trips.
        // (Mixed-strength fixtures can trip the completion guardrail —
        // a guardrail violation is a Regression regardless of block
        // count — so this uses a uniform small benefit: median +2% is
        // below the 5% primary, and no slowdown appears.)
        let few: Vec<Block> = (1..=3)
            .map(|id| Block {
                block_id: id,
                baseline: RunEvents {
                    run_id: format!("a-{id}"),
                    events: measured(&[100.0; 20]),
                },
                treatment: RunEvents {
                    run_id: format!("b-{id}"),
                    events: measured(&[98.0; 20]),
                },
            })
            .collect();
        let report = analyze(
            &few,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &no_safety,
        );
        assert!(report.summary.exploratory);
        assert_eq!(report.label, OutcomeLabel::Inconclusive);
        // The mixed fixture (improvements +10/−5/+10) has a bootstrap
        // slowdown upper bound of 5%: the 2% foreground guardrail makes it
        // a Regression even though the block count is exploratory —
        // guardrails trump small-sample uncertainty.
        let mixed: Vec<Block> = fixture_blocks().into_iter().take(3).collect();
        let report = analyze(
            &mixed,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &no_safety,
        );
        assert_eq!(
            report.label,
            OutcomeLabel::Regression,
            "reasons: {:?}",
            report.reasons
        );
        assert!(
            report.reasons.iter().any(|r| r.contains("guardrail")),
            "reasons: {:?}",
            report.reasons
        );

        // Established regression: every pair −5%, 5 blocks.
        let worse: Vec<Block> = (1..=5)
            .map(|id| Block {
                block_id: id,
                baseline: RunEvents {
                    run_id: format!("a-{id}"),
                    events: measured(&[100.0; 20]),
                },
                treatment: RunEvents {
                    run_id: format!("b-{id}"),
                    events: measured(&[110.0; 20]),
                },
            })
            .collect();
        let report = analyze(
            &worse,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &no_safety,
        );
        assert_eq!(report.label, OutcomeLabel::Regression);

        // Adequate protocol, no meaningful gain (exactly 0% everywhere):
        // NoMeasuredImprovement, not a regression.
        let flat: Vec<Block> = (1..=5)
            .map(|id| Block {
                block_id: id,
                baseline: RunEvents {
                    run_id: format!("a-{id}"),
                    events: measured(&[100.0; 20]),
                },
                treatment: RunEvents {
                    run_id: format!("b-{id}"),
                    events: measured(&[100.0; 20]),
                },
            })
            .collect();
        let report = analyze(
            &flat,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &guardrails,
            &no_safety,
        );
        assert_eq!(report.label, OutcomeLabel::NoMeasuredImprovement);
    }

    #[test]
    fn guardrail_violation_is_a_regression() {
        // Every pair is a 3% slowdown (−3% improvement): the foreground
        // guardrail (2%) is violated even though the interval is tight.
        let blocks: Vec<Block> = (1..=5)
            .map(|id| Block {
                block_id: id,
                baseline: RunEvents {
                    run_id: format!("a-{id}"),
                    events: measured(&[100.0; 20]),
                },
                treatment: RunEvents {
                    run_id: format!("b-{id}"),
                    events: measured(&[103.0; 20]),
                },
            })
            .collect();
        let report = analyze(
            &blocks,
            &BootstrapConfig {
                resamples: 1_000,
                seed: 7,
            },
            &Guardrails::default(),
            &SafetyReport {
                safety_failure: false,
                reasons: vec![],
            },
        );
        assert_eq!(report.label, OutcomeLabel::Regression);
        assert!(report.reasons[0].contains("guardrail"));
    }

    #[test]
    fn cohort_p95_is_never_the_average_of_run_percentiles() {
        // Two runs: p95s 10 ms and 100 ms. A pooled cohort p95 by nearest
        // rank over all events is NOT (10+100)/2 = 55.
        let events: Vec<EventValue> = std::iter::once(EventValue::Measured(10.0))
            .chain((0..9).map(|_| EventValue::Measured(10.0)))
            .chain(std::iter::once(EventValue::Measured(100.0)))
            .chain((0..9).map(|_| EventValue::Measured(100.0)))
            .collect();
        let samples: Vec<f64> = events
            .iter()
            .filter_map(|event| match event {
                EventValue::Measured(value) => Some(*value),
                EventValue::Missing { .. } => None,
            })
            .collect();
        let pooled = nearest_rank(&samples, 0.95).expect("pooled");
        assert!(
            (pooled - 100.0).abs() < f64::EPSILON,
            "pooled p95 must be an event value ({pooled}), never an average of run percentiles"
        );
    }
}
