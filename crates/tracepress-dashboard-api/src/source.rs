use std::{fs, io, path::Path};

use serde_json::Value;
use tracepress_dashboard_types::{
    SourceOptimizationArm, SourceOptimizationSource, SourceOptimizationSummary,
};

const DIRECTORY: &str = "source-passthrough-aa-001";
const REPORTS: [&str; 2] = [
    "TRACEPRESS_EXPLICIT_SHADOW_RESULTS_001.json",
    "TRACEPRESS_EXPLICIT_SHADOW_SMOKE_001.json",
];

pub(crate) fn load(root: &Path) -> io::Result<Option<SourceOptimizationSummary>> {
    let Some(path) = REPORTS
        .iter()
        .map(|name| root.join(DIRECTORY).join(name))
        .find(|path| path.is_file())
    else {
        return Ok(None);
    };
    let report: Value = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let rows = report
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "source report has no rows"))?;
    let treatment: Vec<_> = rows
        .iter()
        .filter(|row| row.get("arm").and_then(Value::as_str) == Some("ExplicitShadow"))
        .collect();
    let raw = sum(&treatment, "source_emitted_bytes");
    let candidate = sum(&treatment, "candidate_bytes");
    let reduction_basis_points = (raw > 0 && candidate <= raw).then(|| {
        u16::try_from(raw.saturating_sub(candidate).saturating_mul(10_000) / raw).unwrap_or(10_000)
    });
    let downstream = ["ExplicitControl", "ExplicitShadow"]
        .into_iter()
        .map(|arm| arm_summary(arm, rows))
        .collect();
    let decision = match report.pointer("/analysis/decision").and_then(Value::as_str) {
        Some("pass") => "pass",
        Some("reject") => "reject",
        _ => "pending",
    };
    Ok(Some(SourceOptimizationSummary {
        experiment_id: "source-passthrough-aa-001".to_owned(),
        decision: decision.to_owned(),
        pairs: report.get("pairs").and_then(Value::as_u64).unwrap_or(0),
        command_family: "cargo_test".to_owned(),
        source: SourceOptimizationSource {
            executions: u64::try_from(treatment.len()).unwrap_or(u64::MAX),
            raw_output_bytes: raw,
            candidate_output_bytes: candidate,
            reduction_basis_points,
            shadow_evaluations: sum(&treatment, "shadow_evaluations"),
            never_worse_accepted: sum(&treatment, "never_worse_accepted"),
            recovery_rate_basis_points: None,
            recovery_hint_bytes: sum(&treatment, "recovery_hint_bytes"),
            reducer_duration_us: sum(&treatment, "reducer_duration_us"),
            forwarding_mutations: 0,
        },
        downstream,
    }))
}

fn arm_summary(arm: &str, rows: &[Value]) -> SourceOptimizationArm {
    let selected: Vec<_> = rows
        .iter()
        .filter(|row| row.get("arm").and_then(Value::as_str) == Some(arm))
        .collect();
    SourceOptimizationArm {
        arm: arm.to_owned(),
        sessions: u64::try_from(selected.len()).unwrap_or(u64::MAX),
        successful_sessions: u64::try_from(
            selected
                .iter()
                .filter(|row| {
                    row.get("agent_exit_status_class").and_then(Value::as_str) == Some("success")
                })
                .count(),
        )
        .unwrap_or(u64::MAX),
        provider_requests: sum(&selected, "provider_requests"),
        input_total: sum_pointer(&selected, "/provider_usage/input_total"),
        cached_input: sum_pointer(&selected, "/provider_usage/input_cached"),
        uncached_input: sum_pointer(&selected, "/provider_usage/input_uncached"),
        output: sum_pointer(&selected, "/provider_usage/output"),
        reasoning: sum_pointer(&selected, "/provider_usage/reasoning"),
        tool_calls: sum(&selected, "source_executions"),
        command_retries: Some(0),
        recovery_requests: 0,
        duration_ms: sum(&selected, "duration_ms"),
    }
}

fn sum(rows: &[&Value], key: &str) -> u64 {
    rows.iter()
        .filter_map(|row| row.get(key).and_then(Value::as_u64))
        .fold(0, u64::saturating_add)
}

fn sum_pointer(rows: &[&Value], pointer: &str) -> u64 {
    rows.iter()
        .filter_map(|row| row.pointer(pointer).and_then(Value::as_u64))
        .fold(0, u64::saturating_add)
}
