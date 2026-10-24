//! Text presentation (JSON is derived serialization on `SystemSnapshot`).
//!
//! MiB/GiB formatting exists only here, at the presentation boundary; all
//! internal values and JSON output are exact bytes. Text output states units
//! and unavailable reasons explicitly (contract 14 §8).

use crate::envelope::{LowMemorySignal, MetricEnvelope};
use crate::snapshot::SystemSnapshot;
use crate::units::format_bytes;

/// Render the `info` command's human-readable snapshot.
#[must_use]
pub fn render_info_text(snapshot: &SystemSnapshot) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str("RAMShaper system memory snapshot\n");
    out.push_str(&format!("  schema          : {}\n", snapshot.schema));
    out.push_str(&format!(
        "  observed (UTC)  : {}\n",
        snapshot.generated_at_utc
    ));
    out.push_str(&format!(
        "  agent instance  : {}\n",
        snapshot.agent_instance_id
    ));
    out.push_str(&format!(
        "  sample sequence : {}\n",
        snapshot.sample_sequence
    ));
    out.push_str(&format!(
        "  tool            : {} {}\n",
        snapshot.tool.name, snapshot.tool.version
    ));
    out.push_str(&format!("  OS              : {}\n", render_os(snapshot)));

    let m = &snapshot.metrics;
    out.push_str("\nPhysical memory (source: GetPerformanceInfo, pages converted to bytes)\n");
    push_byte_metric(&mut out, "total", &m.physical_total, false);
    push_byte_metric(&mut out, "available", &m.physical_available, false);
    push_byte_metric(&mut out, "in use", &m.physical_in_use, true);

    out.push_str("\nSystem commit (source: GetPerformanceInfo; system commit charge, not pagefile occupancy)\n");
    push_byte_metric(&mut out, "used", &m.commit_used, false);
    push_byte_metric(&mut out, "limit", &m.commit_limit, false);
    push_byte_metric(&mut out, "headroom", &m.commit_headroom, true);

    out.push_str("\nLow-memory notification (source: QueryMemoryResourceNotification)\n");
    out.push_str(&format!(
        "  state           : {}\n",
        render_low_memory(&m.low_memory)
    ));

    out.push_str(
        "\nNotes: 'available' is not merely unused pages. Commit is the system commit\n\
         charge, not pagefile occupancy. A low-memory signal is an OS resource\n\
         condition and does not by itself prove a user-visible slowdown.\n",
    );
    out
}

/// Render one watch tick as a single text line.
#[must_use]
pub fn render_watch_line(snapshot: &SystemSnapshot) -> String {
    let m = &snapshot.metrics;
    let low = render_low_memory(&m.low_memory);
    let phys = short_bytes(&m.physical_available);
    let commit = match (&m.commit_used.value, &m.commit_limit.value) {
        (Some(used), Some(limit)) => format!("{}/{}", format_bytes(*used), format_bytes(*limit)),
        _ => "unavailable".to_string(),
    };
    format!(
        "{}  seq {}  low-memory: {}  physical available: {phys}  commit used/limit: {commit}\n",
        snapshot.generated_at_utc, snapshot.sample_sequence, low
    )
}

fn short_bytes(envelope: &MetricEnvelope) -> String {
    match envelope.value {
        Some(bytes) => format_bytes(bytes),
        None => "unavailable".to_string(),
    }
}

fn push_byte_metric(out: &mut String, label: &str, envelope: &MetricEnvelope, derived: bool) {
    let suffix = if derived { " (derived)" } else { "" };
    match envelope.value {
        Some(bytes) => {
            let note = envelope
                .detail
                .as_deref()
                .filter(|_| derived)
                .map_or(String::new(), |d| format!("  [{d}]"));
            out.push_str(&format!(
                "  {:<15}: {:>10} ({bytes} bytes){suffix}{note}\n",
                label,
                format_bytes(bytes)
            ));
        }
        None => {
            out.push_str(&format!(
                "  {:<15}: unavailable{suffix} [{}: {}]\n",
                label,
                envelope.status,
                envelope
                    .detail
                    .clone()
                    .unwrap_or_else(|| "no reason available".to_string())
            ));
        }
    }
}

fn render_low_memory(envelope: &MetricEnvelope) -> String {
    match envelope.state {
        Some(LowMemorySignal::Signaled) => "signaled".to_string(),
        Some(LowMemorySignal::NotSignaled) => "not signaled".to_string(),
        None => format!(
            "unavailable [{}: {}]",
            envelope.status,
            envelope
                .detail
                .clone()
                .unwrap_or_else(|| "no reason available".to_string())
        ),
    }
}

fn render_os(snapshot: &SystemSnapshot) -> String {
    let os = &snapshot.os;
    let product = os
        .product_name
        .clone()
        .unwrap_or_else(|| "unknown product".to_string());
    let version = os
        .version
        .clone()
        .unwrap_or_else(|| "unknown version".to_string());
    let display = os
        .display_version
        .as_deref()
        .map_or(String::new(), |d| format!(", {d}"));
    let arch = os.arch.as_deref().unwrap_or("unknown arch");
    format!(
        "{product} {version}{display}, {arch} (source: {})",
        os.source
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::LowMemorySignal;
    use crate::snapshot::{
        build_snapshot, metric_names, GlobalMemoryRaw, OsInfo, QueryFailure, SnapshotInputs,
        SystemSample, SystemSnapshot, ToolInfo,
    };

    fn raw() -> GlobalMemoryRaw {
        GlobalMemoryRaw {
            physical_total_pages: 4_194_304,
            physical_available_pages: 1_048_576,
            commit_total_pages: 3_145_728,
            commit_limit_pages: 8_388_608,
            page_size_bytes: 4096,
        }
    }

    fn snapshot(low_memory: Result<LowMemorySignal, QueryFailure>) -> SystemSnapshot {
        build_snapshot(SnapshotInputs {
            sample: SystemSample {
                memory: Ok(raw()),
                low_memory,
            },
            os: OsInfo {
                product_name: Some("Windows 11 Pro (synthetic fixture)".to_string()),
                version: Some("10.0.26200.9711".to_string()),
                build: Some(26200),
                ubr: Some(9711),
                display_version: Some("25H2".to_string()),
                arch: Some("x86_64".to_string()),
                source: "synthetic",
            },
            tool: ToolInfo {
                name: "ramshaper-cli".to_string(),
                version: "0.1.0".to_string(),
            },
            agent_instance_id: "cli-4242-133000000000000000 (synthetic)".to_string(),
            sequence: 3,
            observed_at_unix_ms: 1_792_672_496_789,
            monotonic_ms: 5,
        })
    }

    #[test]
    fn info_text_states_units_and_values() {
        let text = render_info_text(&snapshot(Ok(LowMemorySignal::NotSignaled)));
        assert!(text.contains("16.00 GiB (17179869184 bytes)"), "{text}");
        assert!(text.contains("4.00 GiB (4294967296 bytes)"));
        assert!(text.contains("12.00 GiB (12884901888 bytes)"));
        assert!(text.contains("20.00 GiB (21474836480 bytes)"));
        assert!(text.contains("state           : not signaled"));
        assert!(text.contains("does not by itself prove a user-visible slowdown"));
        assert!(text.contains("system commit charge, not pagefile occupancy"));
        assert!(text.contains("sample sequence : 3"));
    }

    #[test]
    fn info_text_shows_unavailable_reasons() {
        let text = render_info_text(&snapshot(Err(QueryFailure::AccessDenied)));
        assert!(
            text.contains("state           : unavailable [access_denied:"),
            "{text}"
        );
        let mut snap = snapshot(Ok(LowMemorySignal::NotSignaled));
        snap.metrics.physical_available.value = None;
        snap.metrics.physical_available.status = crate::status::MetricStatus::AccessDenied;
        snap.metrics.physical_available.error_code = Some("5".into());
        snap.metrics.physical_available.detail = Some("denied".into());
        let text = render_info_text(&snap);
        assert!(text.contains("unavailable [access_denied:"), "{text}");
    }

    #[test]
    fn watch_line_is_single_line_with_state() {
        let line = render_watch_line(&snapshot(Ok(LowMemorySignal::Signaled)));
        assert_eq!(line.matches('\n').count(), 1);
        assert!(line.starts_with("2026-10-22T12:34:56.789Z  seq 3  low-memory: signaled"));
        assert!(line.contains("physical available: 4.00 GiB"));
        assert!(line.contains("commit used/limit: 12.00 GiB/32.00 GiB"));
    }

    #[test]
    fn watch_line_handles_unavailable_commit() {
        let mut snap = snapshot(Ok(LowMemorySignal::NotSignaled));
        snap.metrics.commit_used.value = None;
        let line = render_watch_line(&snap);
        assert!(line.contains("commit used/limit: unavailable"), "{line}");
    }

    #[test]
    fn metric_names_are_stable_json_keys() {
        assert_eq!(metric_names::PHYSICAL_TOTAL, "physical_total_bytes");
        assert_eq!(metric_names::LOW_MEMORY, "low_memory");
    }
}
