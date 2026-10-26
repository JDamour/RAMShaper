//! RS-026 bench-boundary probes: the preregistration contract — a frozen
//! plan is the authority for the executed trial, unknown fields and
//! structural violations refuse at parse, and every deviation is named.
//! In-process (the `prereg` + `analysis` modules are the crate's library
//! surface); the binary-level refusals are captured in the evidence dir.

use ramshaper_bench::analysis::{BootstrapConfig, Guardrails, MIN_BLOCKS_FOR_CONFIRMATORY};
use ramshaper_bench::prereg::{verify_analysis, verify_workload, Preregistration, PREREG_SCHEMA};
use ramshaper_bench::workload::WorkloadSpec;

/// A minimal plan that freezes the workload defaults and the analysis
/// defaults this build actually executes.
fn plan_text() -> String {
    format!(
        r#"{{
        "schema": "{PREREG_SCHEMA}",
        "declared_at_utc": "2026-10-26T21:20:00Z",
        "workload_declaration": "Boundary-review probe: the declaration must be nonempty before the trial.",
        "workload": {{
            "events_per_run": 100,
            "warmup_events": 10,
            "task_iters": 200000,
            "pilot_blocks": 2,
            "order_seed": 12317062951898631936
        }},
        "bootstrap": {{
            "resamples": 10000,
            "seed": 42
        }},
        "thresholds": {{
            "primary_benefit_pct": 5.0,
            "foreground_max_slowdown_pct": 10.0,
            "background_max_slowdown_pct": 25.0,
            "min_blocks_confirmatory": 5
        }}
    }}"#
    )
}

fn parsed_plan() -> Preregistration {
    Preregistration::parse(&plan_text()).expect("valid plan")
}

#[test]
fn closed_schema_refuses_unknown_fields() {
    let with_typo = plan_text().replace("\"declared_at_utc\"", "\"declared_at_utC\"");
    let error = Preregistration::parse(&with_typo).expect_err("unknown field must refuse");
    assert!(error.contains("unknown field"), "{error}");
    // A typo'd schema marker is a version refusal, not a silent acceptance.
    let wrong_schema = plan_text().replace(PREREG_SCHEMA, "ramshaper.preregistration.v2");
    let error = Preregistration::parse(&wrong_schema).expect_err("wrong schema must refuse");
    assert!(error.contains(PREREG_SCHEMA), "{error}");
}

#[test]
fn structural_rules_refuse_at_parse() {
    // Empty workload declaration (doc 08 §3: declared BEFORE the trial).
    let empty_declaration = plan_text().replace(
        "Boundary-review probe: the declaration must be nonempty before the trial.",
        "  ",
    );
    let error =
        Preregistration::parse(&empty_declaration).expect_err("empty declaration must refuse");
    assert!(error.contains("workload_declaration"), "{error}");
    // Sub-p95 event count.
    let few_events = plan_text().replace("\"events_per_run\": 100", "\"events_per_run\": 99");
    let error = Preregistration::parse(&few_events).expect_err("99 events must refuse");
    assert!(error.contains("events_per_run"), "{error}");
    // Non-positive primary benefit.
    let zero_benefit = plan_text().replace(
        "\"primary_benefit_pct\": 5.0",
        "\"primary_benefit_pct\": 0.0",
    );
    let error = Preregistration::parse(&zero_benefit).expect_err("0% benefit must refuse");
    assert!(error.contains("primary_benefit_pct"), "{error}");
}

#[test]
fn every_workload_deviation_is_named_not_just_the_first() {
    let plan = parsed_plan();
    let spec = WorkloadSpec {
        events_per_run: 200, // deviation 1
        warmup_events: 20,   // deviation 2
        task_iters: 300_000, // deviation 3
        seed: 1,             // deviation 4 (the order-seed re-roll)
    };
    let error = verify_workload(&plan, &spec, 7 /* pilot_blocks: deviation 5 */)
        .expect_err("five deviations must refuse");
    assert_eq!(error.len(), 5, "all five named: {error:?}");
    let named: Vec<&str> = error.iter().map(|d| d.parameter).collect();
    for parameter in [
        "workload.events_per_run",
        "workload.warmup_events",
        "workload.task_iters",
        "workload.pilot_blocks",
        "workload.order_seed",
    ] {
        assert!(named.contains(&parameter), "{parameter} missing: {named:?}");
    }
}

#[test]
fn analysis_deviations_are_named_and_the_match_passes_by_construction() {
    let plan = parsed_plan();
    // Defaults this build executes without a plan: the frozen seed 42 and
    // both frozen guardrail bounds differ from the defaults — every one is
    // named (the plan froze non-default values precisely to catch this).
    let config = BootstrapConfig::default();
    let error = verify_analysis(
        &plan,
        &config,
        &Guardrails::default(),
        MIN_BLOCKS_FOR_CONFIRMATORY,
    )
    .expect_err("the defaults deviate from the frozen plan");
    assert_eq!(error.len(), 3, "{error:?}");
    let named: Vec<&str> = error.iter().map(|d| d.parameter).collect();
    for parameter in [
        "bootstrap.seed",
        "thresholds.foreground_max_slowdown_pct",
        "thresholds.background_max_slowdown_pct",
    ] {
        assert!(named.contains(&parameter), "{parameter} missing: {named:?}");
    }

    // The plan-honoring construction (what the fixed tools now do):
    // config and guardrails built FROM the plan pass verification with
    // the fixed min-blocks constant.
    let honored = BootstrapConfig {
        resamples: plan.bootstrap.resamples,
        seed: plan.bootstrap.seed,
    };
    let guardrails = Guardrails {
        foreground_max_slowdown_pct: plan.thresholds.foreground_max_slowdown_pct,
        background_max_slowdown_pct: plan.thresholds.background_max_slowdown_pct,
    };
    verify_analysis(&plan, &honored, &guardrails, MIN_BLOCKS_FOR_CONFIRMATORY)
        .expect("the plan-honoring construction must verify");
}

#[test]
fn a_plan_freezing_a_nondefault_confirmatory_threshold_is_refused() {
    let plan = parsed_plan();
    let honored = BootstrapConfig {
        resamples: plan.bootstrap.resamples,
        seed: plan.bootstrap.seed,
    };
    let guardrails = Guardrails {
        foreground_max_slowdown_pct: plan.thresholds.foreground_max_slowdown_pct,
        background_max_slowdown_pct: plan.thresholds.background_max_slowdown_pct,
    };
    // The build's confirmatory threshold is fixed (MIN_BLOCKS_FOR_CONFIRMATORY);
    // freezing another value is refused loudly on both tools — never
    // silently ignored and never silently honored.
    let error = verify_analysis(
        &plan,
        &honored,
        &guardrails,
        plan.thresholds.min_blocks_confirmatory + 1,
    )
    .expect_err("a mismatched min_blocks_confirmatory must refuse");
    assert_eq!(error[0].parameter, "thresholds.min_blocks_confirmatory");
}

#[test]
fn order_seed_reroll_is_a_named_deviation() {
    // The 0.3.38 contract: re-rolling the AB/BA order randomization after
    // peeking is refused — the seed is a workload deviation like any
    // other, reported with planned vs attempted values.
    let plan = parsed_plan();
    let spec = WorkloadSpec {
        ..WorkloadSpec::default()
    };
    assert_ne!(
        spec.seed, plan.workload.order_seed,
        "fixture uses a different seed"
    );
    let error = verify_workload(&plan, &spec, plan.workload.pilot_blocks)
        .expect_err("a different order seed must refuse");
    let reroll = error
        .iter()
        .find(|d| d.parameter == "workload.order_seed")
        .expect("order_seed deviation named");
    assert_eq!(reroll.planned, plan.workload.order_seed.to_string());
    assert_eq!(reroll.attempted, spec.seed.to_string());
}
