use std::{fs, io, path::Path};

use serde_json::Value;
use tracepress_dashboard_types::{
    SourceOptimizationArm, SourceOptimizationSource, SourceOptimizationSummary,
};

const REPORTS: [(&str, &str); 9] = [
    (
        "source-rg-active-001",
        "TRACEPRESS_RG_ACTIVE_PARSEABLE_RESULTS_001.json",
    ),
    (
        "source-git-status-active-001",
        "TRACEPRESS_GIT_STATUS_ACTIVE_DIRTY_RESULTS_001.json",
    ),
    (
        "source-git-status-shadow-001",
        "TRACEPRESS_GIT_STATUS_SHADOW_RESULTS_001.json",
    ),
    (
        "source-cargo-check-active-001",
        "TRACEPRESS_CHECK_V2_ACTIVE_SUCCESS_RESULTS_001.json",
    ),
    (
        "source-cargo-check-active-001",
        "TRACEPRESS_CHECK_V2_SHADOW_RESULTS_001.json",
    ),
    (
        "source-active-pilot-001",
        "TRACEPRESS_ACTIVE_RESULTS_001.json",
    ),
    (
        "source-active-pilot-001",
        "TRACEPRESS_ACTIVE_SMOKE_001.json",
    ),
    (
        "source-passthrough-aa-001",
        "TRACEPRESS_EXPLICIT_SHADOW_RESULTS_001.json",
    ),
    (
        "source-passthrough-aa-001",
        "TRACEPRESS_EXPLICIT_SHADOW_SMOKE_001.json",
    ),
];

const COMPARISON_REPORTS: [(&str, &str); 5] = [
    (
        "source-active-pilot-001",
        "TRACEPRESS_ACTIVE_RESULTS_001.json",
    ),
    (
        "source-cargo-check-active-001",
        "TRACEPRESS_CHECK_V2_ACTIVE_SUCCESS_RESULTS_001.json",
    ),
    (
        "source-cargo-clippy-shadow-001",
        "TRACEPRESS_CLIPPY_SHADOW_SMOKE_001.json",
    ),
    (
        "source-rg-active-001",
        "TRACEPRESS_RG_ACTIVE_PARSEABLE_RESULTS_001.json",
    ),
    (
        "source-git-status-active-001",
        "TRACEPRESS_GIT_STATUS_ACTIVE_DIRTY_RESULTS_001.json",
    ),
];

pub(crate) fn load(root: &Path) -> io::Result<Option<SourceOptimizationSummary>> {
    let Some(path) = REPORTS
        .iter()
        .map(|(directory, name)| root.join(directory).join(name))
        .find(|path| path.is_file())
    else {
        return Ok(None);
    };
    parse_report(&path).map(Some)
}

pub(crate) fn load_all(root: &Path) -> io::Result<Vec<SourceOptimizationSummary>> {
    COMPARISON_REPORTS
        .iter()
        .map(|(directory, name)| root.join(directory).join(name))
        .filter(|path| path.is_file())
        .map(|path| parse_report(&path))
        .collect()
}

fn parse_report(path: &Path) -> io::Result<SourceOptimizationSummary> {
    let report: Value = serde_json::from_slice(&fs::read(path)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let rows = report
        .get("rows")
        .and_then(Value::as_array)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "source report has no rows"))?;
    let reducer = report.get("reducer").and_then(Value::as_str);
    let (active, treatment_name, control_name, command_family) = match reducer {
        Some("explicit-rg-active") => (true, "ExplicitRgActive", "ExplicitRgActiveControl", "rg"),
        Some("explicit-git-status-active") => (
            true,
            "ExplicitGitStatusActive",
            "ExplicitGitStatusActiveControl",
            "git_status",
        ),
        Some("explicit-git-status-shadow") => (
            false,
            "ExplicitGitStatusShadow",
            "ExplicitGitStatusControl",
            "git_status",
        ),
        Some("explicit-clippy-shadow") => (
            false,
            "ExplicitClippyShadow",
            "ExplicitClippyControl",
            "cargo_clippy",
        ),
        Some("explicit-check-active-v2") => (
            true,
            "ExplicitCheckV2Active",
            "ExplicitCheckV2Control",
            "cargo_check",
        ),
        Some("explicit-check-shadow-v2") => (
            false,
            "ExplicitCheckV2Shadow",
            "ExplicitCheckV2Control",
            "cargo_check",
        ),
        Some("explicit-active") => (
            true,
            "ExplicitActive",
            "ExplicitActiveControl",
            "cargo_test",
        ),
        _ => (false, "ExplicitShadow", "ExplicitControl", "cargo_test"),
    };
    let treatment: Vec<_> = rows
        .iter()
        .filter(|row| row.get("arm").and_then(Value::as_str) == Some(treatment_name))
        .collect();
    let raw = sum(&treatment, "source_stdout_bytes")
        .saturating_add(sum(&treatment, "source_stderr_bytes"));
    let candidate = if active {
        sum(&treatment, "source_emitted_bytes")
    } else {
        sum(&treatment, "candidate_bytes")
    };
    let reduction_basis_points = (raw > 0 && candidate <= raw).then(|| {
        u16::try_from(raw.saturating_sub(candidate).saturating_mul(10_000) / raw).unwrap_or(10_000)
    });
    let downstream = [control_name, treatment_name]
        .into_iter()
        .map(|arm| arm_summary(arm, rows))
        .collect();
    let decision = match report.pointer("/analysis/decision").and_then(Value::as_str) {
        Some("pass") => "pass",
        Some("reject") => "reject",
        _ => "pending",
    };
    Ok(SourceOptimizationSummary {
        experiment_id: report
            .get("experiment_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown-source-experiment")
            .to_owned(),
        decision: decision.to_owned(),
        pairs: report.get("pairs").and_then(Value::as_u64).unwrap_or(0),
        command_family: command_family.to_owned(),
        mode: if active { "active_pilot" } else { "shadow" }.to_owned(),
        provider_effect_active: active,
        source: SourceOptimizationSource {
            executions: u64::try_from(treatment.len()).unwrap_or(u64::MAX),
            raw_output_bytes: raw,
            candidate_output_bytes: candidate,
            reduction_basis_points,
            evaluations: sum(
                &treatment,
                if active {
                    "active_evaluations"
                } else {
                    "shadow_evaluations"
                },
            ),
            never_worse_accepted: sum(&treatment, "never_worse_accepted"),
            recovery_rate_basis_points: active.then(|| {
                let mutations = sum(&treatment, "forwarding_mutations");
                u16::try_from(
                    sum(&treatment, "recovery_requests")
                        .saturating_mul(10_000)
                        .checked_div(mutations)
                        .unwrap_or(0),
                )
                .unwrap_or(10_000)
            }),
            recovery_hint_bytes: sum(&treatment, "recovery_hint_bytes"),
            reducer_duration_us: sum(&treatment, "reducer_duration_us"),
            forwarding_mutations: sum(&treatment, "forwarding_mutations"),
        },
        downstream,
    })
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
                    row.get("task_success")
                        .and_then(Value::as_bool)
                        .unwrap_or_else(|| {
                            row.get("agent_exit_status_class").and_then(Value::as_str)
                                == Some("success")
                        })
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
        tool_calls: sum(&selected, "tool_calls"),
        command_retries: Some(sum(&selected, "command_retries")),
        recovery_requests: sum(&selected, "recovery_requests"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_git_status_shadow_as_a_non_active_fallback() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let directory = root.path().join("source-git-status-shadow-001");
        fs::create_dir(&directory)?;
        let report = serde_json::json!({
            "experiment_id": "source-git-status-shadow-001",
            "reducer": "explicit-git-status-shadow",
            "pairs": 1,
            "analysis": { "decision": "pass" },
            "rows": [
                {
                    "arm": "ExplicitGitStatusControl",
                    "task_success": true,
                    "source_stdout_bytes": 453,
                    "source_emitted_bytes": 453,
                    "provider_usage": {}
                },
                {
                    "arm": "ExplicitGitStatusShadow",
                    "task_success": true,
                    "source_stdout_bytes": 453,
                    "source_emitted_bytes": 453,
                    "candidate_bytes": 203,
                    "shadow_evaluations": 1,
                    "never_worse_accepted": 1,
                    "provider_usage": {}
                }
            ]
        });
        fs::write(
            directory.join("TRACEPRESS_GIT_STATUS_SHADOW_RESULTS_001.json"),
            serde_json::to_vec(&report)?,
        )?;
        let summary = load(root.path())?.ok_or("git status fallback report must load")?;
        assert_eq!(summary.command_family, "git_status");
        assert_eq!(summary.mode, "shadow");
        assert!(!summary.provider_effect_active);
        assert_eq!(summary.source.raw_output_bytes, 453);
        assert_eq!(summary.source.candidate_output_bytes, 203);
        assert_eq!(summary.source.forwarding_mutations, 0);
        Ok(())
    }
}
