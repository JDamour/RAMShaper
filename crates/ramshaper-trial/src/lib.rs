//! `ramshaper-trial` library: the Stage B trial driver's sequencing and
//! wire client, behind the [`AgentRpc`] seam so the trial flow is
//! testable end to end without fabricating OS memory pressure (the gate
//! honestly refuses previews without fresh Elevated evidence — the
//! apply/restore mechanics themselves are proven live in the agent
//! crate's tests, which inject the pressure level through the in-process
//! gate API; the shipped runbook establishes real pressure at trial
//! time).

#![deny(unsafe_code)]

use std::io::{BufRead as _, Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use ramshaper_bench::analysis::{Block, EventValue, RunEvents};
use ramshaper_core::ipc::{
    ConfirmActionRequest, PreviewActionRequest, RequestEnvelope, RestoreActionRequest,
    COMMAND_CONFIRM_ACTION, COMMAND_PREVIEW_ACTION, COMMAND_RESTORE_ACTION, PROTOCOL_VERSION,
};
use ramshaper_subject::{load_spec, SUBJECT_READY_MARKER};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TOOL_NAME: &str = "ramshaper-trial";

/// The CLI help (the binary prints it; `parse_args` handles the flags).
pub const HELP: &str = "\
ramshaper-trial — Stage B trial driver (counterbalanced subject runs behind the agent's consent gate)

USAGE:
    ramshaper-trial --pipe-name <NAME> --subject <PATH> --spec <SPEC.json>
                    [--blocks <N>] [--seed <S>] [--out <FILE>]
                    [--run-prefix <P>] [--exit-delay-ms <MS>]

    Runs N counterbalanced A/B blocks of the subject workload. The A
    (baseline) runs are observe-only. The B (treatment) runs are wrapped
    in the agent's shipped consent machinery over the wire: Preview →
    Confirm (the subject is set to Below Normal) → the measured run →
    Restore. A refusal at any step fails the trial fast — nothing is
    retried and no partial blocks file is written.

    The agent's policy session must already be enabled (typed console
    consent) and must accept the subject (after ADR-045:
    --allowlist-image ramshaper-subject.exe). The driver never enables,
    consents or writes OS settings itself.

OPTIONS:
    --pipe-name <NAME>   The agent's pipe name
    --subject <PATH>     Path to ramshaper-subject.exe
    --spec <SPEC.json>   The workload spec (same JSON the subject takes)
    --blocks <N>         Number of counterbalanced blocks (default 1)
    --seed <S>           Recorded seed for the per-block order coin
                         (default 0x5EED)
    --out <FILE>         Where to write the blocks JSON (default:
                         trial-blocks.json)
    --run-prefix <P>     Prefix for run ids (default \"trial\")
    --exit-delay-ms <MS> Passed to the subject: stay-alive time after the
                         record so the restore lands while it runs
                         (default 800)
    --version, -V        Print version
    --help, -h           Print this help

EXIT CODES:
    0  all blocks executed and the blocks file was written
    1  a run or a consent-gate step failed (the trial stopped honestly)
    2  usage error
";

/// The trial driver's view of the agent: the three policy verbs over the
/// wire. Production uses the named-pipe client; tests use a fake (the
/// gate-side apply/restore with real Elevated evidence is covered by the
/// agent crate's own live tests).
pub trait AgentRpc {
    /// One wire round-trip; `Err` carries the typed refusal.
    fn call(
        &self,
        request_id: &str,
        command: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String>;
}

/// The production [`AgentRpc`]: one fresh named-pipe connection per call
/// (the UI's client pattern).
pub struct PipeRpc<'a> {
    pub pipe_name: &'a str,
}

impl AgentRpc for PipeRpc<'_> {
    fn call(
        &self,
        request_id: &str,
        command: &str,
        payload: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        // The same normalization the agent applies on both of its sides:
        // a bare name is the documented CLI form ("error 123 otherwise" —
        // the recorded Win32 gotcha).
        const PREFIX: &str = "\\\\.\\pipe\\";
        let name = if self.pipe_name.starts_with(PREFIX) {
            self.pipe_name.to_string()
        } else {
            format!("{PREFIX}{}", self.pipe_name)
        };
        rpc(&name, request_id, command, payload)
    }
}

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub pipe_name: String,
    pub subject: PathBuf,
    pub spec: PathBuf,
    pub blocks: u32,
    pub seed: u64,
    pub out: PathBuf,
    pub run_prefix: String,
    pub exit_delay_ms: u64,
}

pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Parsed, String> {
    let mut pipe_name: Option<String> = None;
    let mut subject: Option<PathBuf> = None;
    let mut spec: Option<PathBuf> = None;
    let mut blocks = 1u32;
    let mut seed = 0x5EEDu64;
    let mut out = PathBuf::from("trial-blocks.json");
    let mut run_prefix = "trial".to_string();
    let mut exit_delay_ms = 800u64;
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("{TOOL_NAME} {VERSION}");
                std::process::exit(0);
            }
            "--help" | "-h" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "--pipe-name" => match iter.next() {
                Some(v) => pipe_name = Some(v),
                None => return Err("--pipe-name requires a value".to_string()),
            },
            "--subject" => match iter.next() {
                Some(v) => subject = Some(PathBuf::from(v)),
                None => return Err("--subject requires a path".to_string()),
            },
            "--spec" => match iter.next() {
                Some(v) => spec = Some(PathBuf::from(v)),
                None => return Err("--spec requires a path".to_string()),
            },
            "--blocks" => match iter.next().map(|v| v.parse::<u32>()) {
                Some(Ok(n)) if n >= 1 => blocks = n,
                Some(Ok(n)) => return Err(format!("--blocks {n} must be >= 1")),
                Some(Err(_)) => return Err("--blocks expects a number".to_string()),
                None => return Err("--blocks requires a value".to_string()),
            },
            "--seed" => match iter.next().map(|v| v.parse::<u64>()) {
                Some(Ok(s)) => seed = s,
                Some(Err(_)) => return Err("--seed expects a number".to_string()),
                None => return Err("--seed requires a value".to_string()),
            },
            "--out" => match iter.next() {
                Some(v) => out = PathBuf::from(v),
                None => return Err("--out requires a path".to_string()),
            },
            "--run-prefix" => match iter.next() {
                Some(v) => run_prefix = v,
                None => return Err("--run-prefix requires a value".to_string()),
            },
            "--exit-delay-ms" => match iter.next().map(|v| v.parse::<u64>()) {
                Some(Ok(ms)) if ms <= 60_000 => exit_delay_ms = ms,
                Some(Ok(ms)) => {
                    return Err(format!("--exit-delay-ms {ms} out of range (0..=60000)"))
                }
                Some(Err(_)) => return Err("--exit-delay-ms expects a number".to_string()),
                None => return Err("--exit-delay-ms requires a value".to_string()),
            },
            other => return Err(format!("unknown argument '{other}'")),
        }
    }
    Ok(Parsed {
        pipe_name: pipe_name.ok_or("--pipe-name requires a value")?,
        subject: subject.ok_or("--subject requires a path")?,
        spec: spec.ok_or("--spec requires a path")?,
        blocks,
        seed,
        out,
        run_prefix,
        exit_delay_ms,
    })
}

/// SplitMix64 (the bench's recorded-seed convention; local copy so the
/// driver's order coin is deterministic and independently reproducible).
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The per-block execution order: true = treatment (B) first ("BA").
fn treatment_first(block: u32, seed: u64) -> bool {
    splitmix64(&mut (seed.wrapping_add(block as u64))) & 1 == 1
}

/// One wire round-trip on a fresh connection (the UI's client pattern).
fn rpc(
    pipe_name: &str,
    request_id: &str,
    command: &str,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use ramshaper_windows::pipe::{client_connect, read_frame, write_frame};
    let client =
        client_connect(pipe_name).map_err(|error| format!("agent not reachable: {error}"))?;
    let envelope = RequestEnvelope {
        protocol_version: PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        agent_instance_id: None,
        command: command.to_string(),
        payload,
    };
    let payload =
        serde_json::to_vec(&envelope).map_err(|error| format!("encode failed: {error}"))?;
    write_frame(&client, &payload).map_err(|error| format!("write failed: {error}"))?;
    let frame = read_frame(&client).map_err(|error| format!("read failed: {error}"))?;
    let value: serde_json::Value =
        serde_json::from_slice(&frame).map_err(|error| format!("malformed response: {error}"))?;
    if value["ok"] == true {
        Ok(value["data"].clone())
    } else {
        let code = value["error"]["code"].as_str().unwrap_or("unknown");
        let message = value["error"]["message"].as_str().unwrap_or("");
        Err(format!("{code}: {message}"))
    }
}

/// One executed run: spawn the held subject, (treatment: apply through
/// the gate), release, collect its record, (treatment: restore).
pub fn run_once(
    rpc: &dyn AgentRpc,
    parsed: &Parsed,
    run_id: &str,
    treated: bool,
) -> Result<RunEvents, String> {
    let mut child = Command::new(&parsed.subject)
        .args([
            "--spec",
            &parsed.spec.to_string_lossy(),
            "--run-id",
            run_id,
            "--hold-until-go",
            "--exit-delay-ms",
            &parsed.exit_delay_ms.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot spawn the subject: {error}"))?;
    let pid = child.id();

    let action_id = format!("trial-{run_id}");
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");

    // Wait for the subject to be held and ready (it prints the ready
    // marker as its first stdout line; anything before it is unexpected
    // but tolerated — only the marker releases the run).
    {
        let mut reader = std::io::BufReader::new(&mut stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => return Err("the subject exited before announcing ready".to_string()),
                Ok(_) => {
                    if line.trim() == SUBJECT_READY_MARKER {
                        break;
                    }
                }
                Err(error) => return Err(format!("cannot read the subject's stdout: {error}")),
            }
        }
    }

    if treated {
        // The exact target identity, resolved fresh through the OS.
        let identity = ramshaper_windows::identity::resolve_identity(pid)
            .map_err(|error| format!("cannot resolve the subject's identity: {error}"))?;
        let preview = rpc_call(
            rpc,
            &format!("{action_id}-preview"),
            COMMAND_PREVIEW_ACTION,
            Some(
                serde_json::to_value(PreviewActionRequest {
                    action_id: action_id.clone(),
                    pid,
                    creation_filetime: identity.creation_filetime_100ns.to_string(),
                    image_name: "ramshaper-subject.exe".to_string(),
                    ttl_ms: None,
                })
                .map_err(|error| format!("preview encode failed: {error}"))?,
            ),
        )?;
        if preview["verdict"] != "offered" {
            return Err(format!("preview was not offered: {preview}"));
        }
        let confirm = rpc_call(
            rpc,
            &format!("{action_id}-confirm"),
            COMMAND_CONFIRM_ACTION,
            Some(
                serde_json::to_value(ConfirmActionRequest {
                    action_id: action_id.clone(),
                })
                .map_err(|error| format!("confirm encode failed: {error}"))?,
            ),
        )?;
        if confirm["outcome"] != "applied" {
            return Err(format!("confirm did not apply: {confirm}"));
        }
        eprintln!("treatment applied for {run_id} (action {action_id})");
    }

    // Release the subject and collect its record.
    stdin
        .write_all(b"GO\n")
        .map_err(|error| format!("cannot release the subject: {error}"))?;
    drop(stdin);
    let mut record_text = String::new();
    stdout
        .read_to_string(&mut record_text)
        .map_err(|error| format!("cannot read the subject's record: {error}"))?;
    let status = child
        .wait()
        .map_err(|error| format!("wait failed: {error}"))?;
    if !status.success() {
        return Err(format!("the subject exited {status}"));
    }
    let record: serde_json::Value = serde_json::from_str(record_text.trim())
        .map_err(|error| format!("the subject's record is not valid JSON: {error}"))?;
    if record["completion"]["status"] != "complete" {
        return Err(format!("the subject's run did not complete: {record}"));
    }
    let latencies: Vec<EventValue> = record["latencies_ms"]
        .as_array()
        .ok_or("the record carries no latency series")?
        .iter()
        .map(|value| match value.as_f64() {
            Some(ms) => EventValue::Measured(ms),
            None => EventValue::Missing {
                reason: format!("subject recorded a non-numeric latency: {value}"),
            },
        })
        .collect();

    if treated {
        let restore = rpc_call(
            rpc,
            &format!("{action_id}-restore"),
            COMMAND_RESTORE_ACTION,
            Some(
                serde_json::to_value(RestoreActionRequest {
                    action_id: action_id.clone(),
                })
                .map_err(|error| format!("restore encode failed: {error}"))?,
            ),
        )?;
        // The subject is still alive here (--exit-delay-ms), so a clean
        // "restored" is the expected outcome; anything else is recorded
        // and fails the trial (never silently accepted).
        let outcome = restore["outcome"].as_str().unwrap_or("unknown");
        if outcome != "restored" {
            return Err(format!("restore outcome was '{outcome}': {restore}"));
        }
        eprintln!("treatment restored for {run_id}");
    }

    Ok(RunEvents {
        run_id: run_id.to_string(),
        events: latencies,
    })
}

pub fn run_trial(rpc: &dyn AgentRpc, parsed: &Parsed) -> Result<Vec<Block>, String> {
    let spec = load_spec(&parsed.spec)?;
    spec.validate()
        .map_err(|error| format!("invalid spec: {error}"))?;
    let mut blocks = Vec::with_capacity(parsed.blocks as usize);
    for block in 1..=parsed.blocks {
        let treated_first = treatment_first(block, parsed.seed);
        let conditions: [(&str, bool); 2] = if treated_first {
            [("B", true), ("A", false)]
        } else {
            [("A", false), ("B", true)]
        };
        let mut baseline: Option<RunEvents> = None;
        let mut treatment: Option<RunEvents> = None;
        for (condition, treated) in conditions {
            let run_id = format!("{}b{block}-{condition}", parsed.run_prefix);
            eprintln!(
                "block {block} run {run_id} ({}): starting",
                if treated { "treatment" } else { "baseline" }
            );
            let events = run_once(rpc, parsed, &run_id, treated)?;
            println!(
                "block {block} {condition}: {} events recorded",
                events.events.len()
            );
            let _ = std::io::stdout().flush();
            if treated {
                treatment = Some(events);
            } else {
                baseline = Some(events);
            }
        }
        blocks.push(Block {
            block_id: block,
            baseline: baseline.expect("baseline always set"),
            treatment: treatment.expect("treatment always set"),
        });
    }
    Ok(blocks)
}

/// The `analyze-ab` blocks-file wire shape: `analysis::Block` is the
/// INPUT type (Deserialize only), so the emission is field-for-field
/// here — untagged `EventValue` = plain numbers, milliseconds; missing
/// values as `{"reason": ...}` (never silently dropped).
#[must_use]
pub fn blocks_to_wire(blocks: &[Block]) -> Vec<serde_json::Value> {
    blocks
        .iter()
        .map(|block| {
            let run = |run_events: &RunEvents| {
                serde_json::json!({
                    "run_id": run_events.run_id,
                    "events": run_events
                        .events
                        .iter()
                        .map(|value| match value {
                            EventValue::Measured(ms) => serde_json::json!(ms),
                            EventValue::Missing { reason } => {
                                serde_json::json!({ "reason": reason })
                            }
                        })
                        .collect::<Vec<_>>(),
                })
            };
            serde_json::json!({
                "block_id": block.block_id,
                "baseline": run(&block.baseline),
                "treatment": run(&block.treatment),
            })
        })
        .collect()
}

/// Route one policy verb through the seam with the standard request id.
fn rpc_call(
    rpc: &dyn AgentRpc,
    request_id: &str,
    command: &str,
    payload: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    rpc.call(request_id, command, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_coin_is_deterministic_and_varies() {
        // Same seed: the same order for every block, reproducibly.
        assert_eq!(treatment_first(1, 0x5EED), treatment_first(1, 0x5EED));
        assert_eq!(treatment_first(2, 0x5EED), treatment_first(2, 0x5EED));
        // Across many seeds both orders occur (the coin is not stuck).
        let ba_count: u32 = (0..256u64).filter(|seed| treatment_first(1, *seed)).count() as u32;
        assert!(
            (16..=240).contains(&ba_count),
            "the order coin should vary across seeds: {ba_count}/256 treatment-first"
        );
    }

    #[test]
    fn usage_rules() {
        assert!(parse_args(vec![]).is_err());
        assert!(parse_args(vec!["--blocks".to_string(), "0".to_string()]).is_err());
        assert!(parse_args(vec!["--bogus".to_string()]).is_err());
        let parsed = parse_args(vec![
            "--pipe-name".to_string(),
            "p".to_string(),
            "--subject".to_string(),
            "s.exe".to_string(),
            "--spec".to_string(),
            "spec.json".to_string(),
            "--blocks".to_string(),
            "3".to_string(),
        ])
        .expect("parses");
        assert_eq!(parsed.blocks, 3);
        assert_eq!(parsed.seed, 0x5EED);
        assert_eq!(parsed.out, PathBuf::from("trial-blocks.json"));
    }
}
