use std::{fs, path::Path};

use serde_json::Value;
use tracepress_dashboard_types::{
    BaselineDetail, BaselineManifest, BaselineSummary, ContextCategoryStats, ContextComposition,
    ConvergencePoint, MeasurementQuality, Metric, MetricSource, OpportunitySummary,
    RepetitionSummary, UsageSummary, WorkloadSummary,
};

pub(crate) fn list(root: &Path) -> Result<Vec<BaselineSummary>, std::io::Error> {
    let mut summaries = Vec::new();
    for number in 1..=4 {
        let id = format!("baseline-{number:03}");
        let directory = root.join(&id);
        if directory.is_dir() {
            summaries.push(read_summary(&directory, &id)?);
        }
    }
    Ok(summaries)
}

pub(crate) fn detail(root: &Path, id: &str) -> Result<Option<BaselineDetail>, std::io::Error> {
    let directory = root.join(id);
    if !directory.is_dir() {
        return Ok(None);
    }
    let summary = read_summary(&directory, id)?;
    let report = read_latest_report(&directory, id)?.unwrap_or(Value::Null);
    let manifest_value =
        read_json(&directory.join("measurement_manifest.json")).or_else(|_error| {
            Ok::<Value, std::io::Error>(report.get("manifest").cloned().unwrap_or(Value::Null))
        })?;
    let manifest = BaselineManifest {
        instrument_sha: string_at(&manifest_value, "measurement_tooling_commit"),
        runtime_sha: string_at(&manifest_value, "runtime_commit")
            .or_else(|| string_at(&manifest_value, "tracepress_commit")),
        codex_version: string_at(&manifest_value, "codex_version"),
        model: string_at(&manifest_value, "model"),
        reasoning_effort: string_at(&manifest_value, "reasoning_effort"),
        transport: string_at(&manifest_value, "transport"),
    };
    let usage = report.get("provider_usage").map(usage_from_report);
    let quality = report.get("quality").map(quality_from_report);
    let composition = report
        .pointer("/composition/detected_content")
        .and_then(Value::as_array)
        .map(|rows| ContextComposition {
            group_by: "detected_kind".to_owned(),
            categories: rows.iter().map(category_from_report).collect(),
            estimator_coverage: report
                .pointer("/quality/token_estimation_coverage/by_blocks")
                .and_then(Value::as_f64),
        });
    let repetition = report.get("repetition").map(|value| RepetitionSummary {
        exact_token_share: estimated_f64(
            value
                .get("exact_repeated_token_share")
                .and_then(Value::as_f64),
        ),
        semantic_token_share: estimated_f64(
            value
                .get("semantic_repeated_token_share")
                .and_then(Value::as_f64),
        ),
        provider_cache_ratio: provider_f64(
            report
                .pointer("/provider_usage/cache_ratio")
                .and_then(Value::as_f64),
        ),
    });
    let workloads = workloads_from_report(&report);
    let convergence = convergence_points(&directory)?;
    let recommendation = report
        .get("recommendation")
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(Some(BaselineDetail {
        summary,
        manifest,
        usage,
        quality,
        composition,
        repetition,
        workloads,
        convergence,
        recommendation,
    }))
}

pub(crate) fn load_workloads(root: &Path) -> Result<Vec<WorkloadSummary>, std::io::Error> {
    let report =
        read_latest_report(&root.join("baseline-004"), "baseline-004")?.unwrap_or(Value::Null);
    Ok(workloads_from_report(&report))
}

pub(crate) fn load_opportunities(root: &Path) -> Result<Vec<OpportunitySummary>, std::io::Error> {
    let report =
        read_latest_report(&root.join("baseline-004"), "baseline-004")?.unwrap_or(Value::Null);
    let repetition = report
        .pointer("/repetition/effective_presence")
        .and_then(Value::as_array);
    let ranking = report.get("opportunity_ranking").and_then(Value::as_array);
    let mut output = Vec::new();
    if let Some(rows) = ranking {
        for row in rows {
            let category = string_at(row, "name").unwrap_or_else(|| "unknown".to_owned());
            let presence = repetition.and_then(|values| {
                values.iter().find(|value| {
                    value.get("name").and_then(Value::as_str) == Some(category.as_str())
                })
            });
            output.push(OpportunitySummary {
                category,
                effective_presence: presence
                    .and_then(|value| u64_at(value, "effective_presence_estimated_tokens")),
                repeated_tokens: presence
                    .and_then(|value| u64_at(value, "repeated_estimated_tokens")),
                persistence: row.get("average_persistence").and_then(Value::as_f64),
                redundancy: row.get("redundancy_factor").and_then(Value::as_f64),
                measurement_confidence: row.get("measurement_confidence").and_then(Value::as_f64),
                candidate_priority: row
                    .get("phase_4_candidate_priority_score")
                    .and_then(Value::as_f64),
            });
        }
    }
    Ok(output)
}

fn read_summary(directory: &Path, id: &str) -> Result<BaselineSummary, std::io::Error> {
    let report = read_latest_report(directory, id)?.unwrap_or(Value::Null);
    let convergence =
        latest_named_json(directory, "convergence_")?.and_then(|path| read_json(&path).ok());
    let status = match id {
        "baseline-001" => "exploratory".to_owned(),
        "baseline-002" => "legacy_uncertified".to_owned(),
        "baseline-003" => "valid".to_owned(),
        _ if convergence
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(Value::as_str)
            == Some("CONVERGED") =>
        {
            "converged".to_owned()
        }
        _ => "valid".to_owned(),
    };
    Ok(BaselineSummary {
        id: id.to_owned(),
        status,
        sessions: report
            .pointer("/dataset/requests_per_session/count")
            .and_then(Value::as_u64),
        requests: report
            .pointer("/dataset/requests_total")
            .and_then(Value::as_u64),
        cohort: report
            .get("cohort_label")
            .and_then(Value::as_str)
            .map(str::to_owned),
        measurement_integrity: report
            .pointer("/quality/measurement_integrity")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn convergence_points(directory: &Path) -> Result<Vec<ConvergencePoint>, std::io::Error> {
    let convergence =
        latest_named_json(directory, "convergence_")?.and_then(|path| read_json(&path).ok());
    let mut points = Vec::new();
    for size in [10_u64, 20, 30, 40] {
        let path = directory.join(format!("baseline_n{size}.json"));
        if !path.is_file() {
            continue;
        }
        let report = read_json(&path)?;
        let detected = report
            .pointer("/composition/detected_content")
            .and_then(Value::as_array);
        let share = |name: &str| {
            detected
                .and_then(|rows| {
                    rows.iter()
                        .find(|row| row.get("name").and_then(Value::as_str) == Some(name))
                })
                .and_then(|row| row.get("token_share"))
                .and_then(Value::as_f64)
        };
        let stable = convergence
            .as_ref()
            .and_then(|value| value.get("transitions"))
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("current_sessions").and_then(Value::as_u64) == Some(size))
            })
            .and_then(|row| row.get("stable"))
            .and_then(Value::as_bool);
        points.push(ConvergencePoint {
            cohort: format!("N{size}"),
            json_share: share("json"),
            plain_share: share("plain_text"),
            unknown_share: share("unknown"),
            cache_ratio: report
                .pointer("/provider_usage/cache_ratio")
                .and_then(Value::as_f64),
            repetition: report
                .pointer("/repetition/exact_repeated_token_share")
                .and_then(Value::as_f64),
            stable,
        });
    }
    Ok(points)
}

fn workloads_from_report(report: &Value) -> Vec<WorkloadSummary> {
    let Some(rows) = report
        .pointer("/composition/by_workload")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let requests_by_workload = report
        .pointer("/provider_usage/by_workload")
        .and_then(Value::as_array);
    rows.iter()
        .map(|row| {
            let workload = string_at(row, "name").unwrap_or_else(|| "unknown".to_owned());
            let detected = row.get("by_detected_content").and_then(Value::as_array);
            let share = |name: &str| {
                detected
                    .and_then(|values| {
                        values
                            .iter()
                            .find(|value| value.get("name").and_then(Value::as_str) == Some(name))
                    })
                    .and_then(|value| value.get("token_share"))
                    .and_then(Value::as_f64)
            };
            let provider = requests_by_workload.and_then(|values| {
                values.iter().find(|value| {
                    value.get("name").and_then(Value::as_str) == Some(workload.as_str())
                })
            });
            WorkloadSummary {
                workload,
                sessions: u64_at(row, "session_count").unwrap_or(0),
                requests: provider
                    .and_then(|value| u64_at(value, "requests"))
                    .unwrap_or(0),
                token_share: row.get("workload_token_share").and_then(Value::as_f64),
                json_share: share("json"),
                plain_share: share("plain_text"),
                unknown_share: share("unknown"),
                cache_ratio: provider
                    .and_then(|value| value.get("cache_ratio"))
                    .and_then(Value::as_f64),
                exact_repetition: None,
                semantic_repetition: None,
            }
        })
        .collect()
}

fn usage_from_report(value: &Value) -> UsageSummary {
    UsageSummary {
        input_tokens: provider_u64(u64_at(value, "input_total")),
        cached_input_tokens: provider_u64(u64_at(value, "cached_input")),
        uncached_input_tokens: provider_u64(u64_at(value, "uncached_input")),
        cache_ratio: provider_f64(value.get("cache_ratio").and_then(Value::as_f64)),
        output_tokens: provider_u64(u64_at(value, "output_total")),
        reasoning_tokens: provider_u64(u64_at(value, "reasoning_output")),
    }
}

fn quality_from_report(value: &Value) -> MeasurementQuality {
    let status = value
        .get("measurement_integrity")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    MeasurementQuality {
        status: if status == "passed" {
            "healthy"
        } else {
            "degraded"
        }
        .to_owned(),
        analysis_coverage: value.get("analysis_coverage").and_then(Value::as_f64),
        correlation_coverage: value.get("correlation_coverage").and_then(Value::as_f64),
        semantic_coverage: value
            .pointer("/semantic_coverage/mean")
            .and_then(Value::as_f64),
        dropped_requests: u64_at(value, "analysis_dropped").unwrap_or(0),
        malformed_requests: u64_at(value, "context_malformed").unwrap_or(0),
        reasons: Vec::new(),
    }
}

fn category_from_report(value: &Value) -> ContextCategoryStats {
    ContextCategoryStats {
        name: string_at(value, "name").unwrap_or_else(|| "unknown".to_owned()),
        block_count: u64_at(value, "block_count").unwrap_or(0),
        raw_bytes: u64_at(value, "bytes")
            .or_else(|| u64_at(value, "raw_bytes"))
            .unwrap_or(0),
        estimated_tokens: u64_at(value, "estimated_tokens"),
        estimated_token_share: value.get("token_share").and_then(Value::as_f64),
        exact_repeated_tokens: None,
        semantic_repeated_tokens: None,
        persistence: None,
    }
}

fn read_latest_report(directory: &Path, id: &str) -> Result<Option<Value>, std::io::Error> {
    let canonical = directory.join(format!(
        "TRACEPRESS_{}.json",
        id.replace('-', "_").to_uppercase()
    ));
    if canonical.is_file() {
        return read_json(&canonical).map(Some);
    }
    latest_named_json(directory, "baseline_n")?.map_or(Ok(None), |path| read_json(&path).map(Some))
}

fn latest_named_json(
    directory: &Path,
    prefix: &str,
) -> Result<Option<std::path::PathBuf>, std::io::Error> {
    let mut files = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("json")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with(prefix))
        })
        .collect::<Vec<_>>();
    files.sort();
    Ok(files.pop())
}

fn read_json(path: &Path) -> Result<Value, std::io::Error> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(std::io::Error::other)
}

fn string_at(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn u64_at(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}
fn provider_u64(value: Option<u64>) -> Metric<u64> {
    Metric::new(
        value,
        if value.is_some() {
            MetricSource::ProviderReported
        } else {
            MetricSource::Unavailable
        },
    )
}
fn provider_f64(value: Option<f64>) -> Metric<f64> {
    Metric::new(
        value.filter(|number| number.is_finite()),
        if value.is_some_and(f64::is_finite) {
            MetricSource::ProviderReported
        } else {
            MetricSource::Unavailable
        },
    )
}
fn estimated_f64(value: Option<f64>) -> Metric<f64> {
    Metric::new(
        value.filter(|number| number.is_finite()),
        if value.is_some_and(f64::is_finite) {
            MetricSource::LocallyEstimated
        } else {
            MetricSource::Unavailable
        },
    )
}
