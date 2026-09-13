use std::{path::PathBuf, sync::Arc, time::Duration};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tracepress_dashboard_types::{
    CompressionCandidateSummary, CompressionExperimentDetail, CompressionExperimentSummary,
    CompressionHistogramBucket, CompressionQuality, CompressorSummary, ContextBlockSummary,
    ContextCategoryStats, ContextComposition, ContextExplorer, ContextGrowthPoint,
    ContextMatrixCell, MeasurementQuality, Metric, MetricSource, Overview, Page, Percentiles,
    ProviderRequestSummary, RepetitionSummary, SessionContext, SessionDetail, SessionSummary,
    UnknownSummary, UsageSummary,
};

use crate::{ApiFailure, SessionsQuery, SessionsSort};

pub(crate) async fn run<T, F>(path: Arc<PathBuf>, operation: F) -> Result<T, ApiFailure>
where
    T: Send + 'static,
    F: FnOnce(&Connection) -> Result<T, rusqlite::Error> + Send + 'static,
{
    tokio::task::spawn_blocking(move || {
        let connection = open_read_only(path.as_ref()).map_err(|_error| ApiFailure::Internal)?;
        operation(&connection).map_err(|_error| ApiFailure::Internal)
    })
    .await
    .map_err(|_error| ApiFailure::Internal)?
}

pub(crate) fn validate_database(path: &std::path::Path) -> Result<(), rusqlite::Error> {
    let connection = open_read_only(path)?;
    let present: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sessions')",
        [],
        |row| row.get(0),
    )?;
    if present {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

fn open_read_only(path: &std::path::Path) -> Result<Connection, rusqlite::Error> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(Duration::from_millis(250))?;
    connection.pragma_update(None, "query_only", true)?;
    Ok(connection)
}

pub(crate) fn health(connection: &Connection) -> Result<(), rusqlite::Error> {
    let value: i64 = connection.query_row("SELECT 1", [], |row| row.get(0))?;
    if value == 1 {
        Ok(())
    } else {
        Err(rusqlite::Error::InvalidQuery)
    }
}

pub(crate) fn overview(connection: &Connection) -> Result<Overview, rusqlite::Error> {
    let sessions = nonnegative(connection.query_row(
        "SELECT COUNT(*) FROM sessions",
        [],
        |row| row.get::<_, i64>(0),
    )?);
    let provider_requests = nonnegative(connection.query_row(
        "SELECT COUNT(*) FROM provider_requests",
        [],
        |row| row.get::<_, i64>(0),
    )?);
    let usage = aggregate_usage(connection, None)?;
    let composition = composition(connection, "detected_kind")?;
    let repetition = repetition_with_cache(connection, usage.cache_ratio.clone())?;
    let quality = quality(connection, provider_requests)?;
    Ok(Overview {
        sessions,
        provider_requests,
        usage,
        composition,
        repetition,
        quality,
    })
}

fn aggregate_usage(
    connection: &Connection,
    session_id: Option<&str>,
) -> Result<UsageSummary, rusqlite::Error> {
    let filter = if session_id.is_some() {
        "WHERE o.session_id = ?1"
    } else {
        ""
    };
    let sql = format!(
        "WITH final_attempt AS (
            SELECT pr.request_id, o.session_id, pa.attempt_id
            FROM provider_requests pr
            JOIN operations o ON o.operation_id = pr.operation_id
            LEFT JOIN provider_attempts pa ON pa.request_id = pr.request_id
                AND pa.ordinal = (SELECT MAX(pa2.ordinal) FROM provider_attempts pa2 WHERE pa2.request_id = pr.request_id)
            {filter}
        )
        SELECT COUNT(*), COUNT(pu.input_total), SUM(pu.input_total),
               COUNT(pu.input_cached), SUM(pu.input_cached),
               COUNT(pu.input_uncached), SUM(pu.input_uncached),
               COUNT(pu.output_total), SUM(pu.output_total),
               COUNT(pu.output_reasoning), SUM(pu.output_reasoning)
        FROM final_attempt fa LEFT JOIN provider_usage pu ON pu.attempt_id = fa.attempt_id"
    );
    let query = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, Option<i64>>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<i64>>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, Option<i64>>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, Option<i64>>(8)?,
            row.get::<_, i64>(9)?,
            row.get::<_, Option<i64>>(10)?,
        ))
    };
    let values = if let Some(id) = session_id {
        connection.query_row(&sql, params![id], query)?
    } else {
        connection.query_row(&sql, [], query)?
    };
    let (
        rows,
        input_rows,
        input,
        cache_rows,
        cached,
        uncached_rows,
        uncached,
        output_rows,
        output,
        reasoning_rows,
        reasoning,
    ) = values;
    let complete = |count: i64, value: Option<i64>| {
        (rows > 0 && count == rows)
            .then(|| value.and_then(to_u64))
            .flatten()
    };
    let input = complete(input_rows, input);
    let cached = complete(cache_rows, cached);
    let uncached = complete(uncached_rows, uncached).or_else(|| {
        input
            .zip(cached)
            .and_then(|(total, cache)| total.checked_sub(cache))
    });
    let ratio = input
        .zip(cached)
        .and_then(|(total, cache)| ratio(cache, total));
    Ok(UsageSummary {
        input_tokens: provider_metric(input),
        cached_input_tokens: provider_metric(cached),
        uncached_input_tokens: provider_metric(uncached),
        cache_ratio: Metric::new(ratio, source_for(ratio, MetricSource::ProviderReported)),
        output_tokens: provider_metric(complete(output_rows, output)),
        reasoning_tokens: provider_metric(complete(reasoning_rows, reasoning)),
    })
}

pub(crate) fn repetition(connection: &Connection) -> Result<RepetitionSummary, rusqlite::Error> {
    let usage = aggregate_usage(connection, None)?;
    repetition_with_cache(connection, usage.cache_ratio)
}

fn repetition_with_cache(
    connection: &Connection,
    cache_ratio: Metric<f64>,
) -> Result<RepetitionSummary, rusqlite::Error> {
    let (total, exact, semantic): (Option<i64>, Option<i64>, Option<i64>) = connection.query_row(
        "WITH frequencies AS (
            SELECT block_occurrence_id, estimated_tokens,
                   COUNT(*) OVER (PARTITION BY exact_fingerprint) exact_count,
                   COUNT(*) OVER (PARTITION BY semantic_fingerprint) semantic_count,
                   exact_fingerprint, semantic_fingerprint
            FROM context_block_occurrences WHERE estimated_tokens IS NOT NULL
        )
        SELECT SUM(estimated_tokens),
               SUM(CASE WHEN exact_fingerprint IS NOT NULL AND exact_count > 1 THEN estimated_tokens ELSE 0 END),
               SUM(CASE WHEN semantic_fingerprint IS NOT NULL AND semantic_count > 1 THEN estimated_tokens ELSE 0 END)
        FROM frequencies",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let exact_ratio = total
        .and_then(to_u64)
        .zip(exact.and_then(to_u64))
        .and_then(|(all, repeated)| ratio(repeated, all));
    let semantic_ratio = total
        .and_then(to_u64)
        .zip(semantic.and_then(to_u64))
        .and_then(|(all, repeated)| ratio(repeated, all));
    Ok(RepetitionSummary {
        exact_token_share: estimated_ratio(exact_ratio),
        semantic_token_share: estimated_ratio(semantic_ratio),
        provider_cache_ratio: cache_ratio,
    })
}

fn quality(
    connection: &Connection,
    provider_requests: u64,
) -> Result<MeasurementQuality, rusqlite::Error> {
    let (complete, partial, dropped, malformed, correlated, semantic_sum, semantic_rows): (i64, i64, i64, i64, i64, Option<i64>, i64) = connection.query_row(
        "SELECT
            COALESCE(SUM(CASE WHEN cs.status = 'complete' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN cs.status = 'partial' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN cs.status = 'dropped' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN cs.logical_context_status = 'malformed' THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE WHEN cs.correlation_status = 'correlated' THEN 1 ELSE 0 END), 0),
            SUM(cam.semantic_coverage_basis_points),
            COUNT(cam.semantic_coverage_basis_points)
         FROM context_snapshots cs
         LEFT JOIN context_analysis_metrics cam ON cam.snapshot_id = cs.snapshot_id
         WHERE cs.analysis_version = (SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2 WHERE cs2.provider_request_id = cs.provider_request_id)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?)),
    )?;
    let observed = nonnegative(complete.saturating_add(partial).saturating_add(dropped));
    let correlation_eligible = observed.saturating_sub(nonnegative(dropped));
    let analysis_coverage = ratio(nonnegative(complete), provider_requests);
    let correlation_coverage = ratio(nonnegative(correlated), correlation_eligible);
    let semantic_coverage = if semantic_rows > 0 {
        semantic_sum.and_then(|sum| finite_ratio(sum as f64, semantic_rows as f64 * 10_000.0))
    } else {
        None
    };
    let mut reasons = Vec::new();
    if provider_requests > nonnegative(complete) {
        reasons.push(format!(
            "{} requests missing complete context analysis",
            provider_requests.saturating_sub(nonnegative(complete))
        ));
    }
    if dropped > 0 {
        reasons.push(format!("{} context analyses dropped", dropped));
    }
    if malformed > 0 {
        reasons.push(format!("{} malformed context observations", malformed));
    }
    Ok(MeasurementQuality {
        status: if reasons.is_empty() {
            "healthy"
        } else {
            "degraded"
        }
        .to_owned(),
        analysis_coverage,
        correlation_coverage,
        semantic_coverage,
        dropped_requests: nonnegative(dropped),
        malformed_requests: nonnegative(malformed),
        reasons,
    })
}

pub(crate) fn sessions(
    connection: &Connection,
    query: &SessionsQuery,
    (limit, offset): (u32, u64),
) -> Result<Page<SessionSummary>, rusqlite::Error> {
    if query.workload.is_some() || query.measurement_id.is_some() {
        return Ok(Page {
            items: Vec::new(),
            next_cursor: None,
        });
    }
    let sort = query
        .sort
        .as_deref()
        .unwrap_or("created_at")
        .parse::<SessionsSort>()
        .map_err(|_error| rusqlite::Error::InvalidQuery)?;
    let sort_sql = match sort {
        SessionsSort::CreatedAt => "started_at",
        SessionsSort::RequestCount => "request_count",
        SessionsSort::InputTokens => "input_tokens",
        SessionsSort::CacheRatio => "cache_ratio",
        SessionsSort::EstimatedContextTokens => "estimated_context_tokens",
        SessionsSort::Repetition => "repetition",
    };
    let direction = match query.direction.as_deref().unwrap_or("desc") {
        "asc" => "ASC",
        "desc" => "DESC",
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let sql = format!(
        "WITH request_data AS (
            SELECT o.session_id, pr.request_id, pr.model, pr.transport, pr.request_kind,
                   pu.input_total, pu.input_cached, pu.input_uncached, pu.output_total, pu.output_reasoning,
                   pa.duration_us,
                   (SELECT SUM(cbo.estimated_tokens) FROM context_snapshots cs JOIN context_block_occurrences cbo ON cbo.snapshot_id = cs.snapshot_id
                    WHERE cs.provider_request_id = pr.request_id AND cs.analysis_version = (SELECT MAX(cs2.analysis_version) FROM context_snapshots cs2 WHERE cs2.provider_request_id = pr.request_id)) estimated_context,
                   (SELECT cam.semantic_coverage_basis_points FROM context_snapshots cs JOIN context_analysis_metrics cam ON cam.snapshot_id = cs.snapshot_id
                    WHERE cs.provider_request_id = pr.request_id ORDER BY cs.analysis_version DESC LIMIT 1) semantic_bp,
                   (SELECT cd.repeated_estimated_tokens FROM context_snapshots cs JOIN context_deltas cd ON cd.current_snapshot_id = cs.snapshot_id
                    WHERE cs.provider_request_id = pr.request_id ORDER BY cs.analysis_version DESC LIMIT 1) repeated_tokens
            FROM provider_requests pr JOIN operations o ON o.operation_id = pr.operation_id
            LEFT JOIN provider_attempts pa ON pa.request_id = pr.request_id AND pa.ordinal = (SELECT MAX(pa2.ordinal) FROM provider_attempts pa2 WHERE pa2.request_id = pr.request_id)
            LEFT JOIN provider_usage pu ON pu.attempt_id = pa.attempt_id
        ), aggregate AS (
            SELECT s.session_id, s.started_at, s.ended_at, s.state,
                   COUNT(rd.request_id) request_count,
                   MAX(rd.model) model, MAX(rd.transport) transport,
                   MAX(CASE WHEN rd.request_kind LIKE 'compaction%' THEN 1 ELSE 0 END) has_compaction,
                   CASE WHEN COUNT(rd.input_total) = COUNT(rd.request_id) AND COUNT(rd.request_id) > 0 THEN SUM(rd.input_total) END input_tokens,
                   CASE WHEN COUNT(rd.input_cached) = COUNT(rd.request_id) AND COUNT(rd.request_id) > 0 THEN SUM(rd.input_cached) END cached_tokens,
                   CASE WHEN COUNT(rd.input_uncached) = COUNT(rd.request_id) AND COUNT(rd.request_id) > 0 THEN SUM(rd.input_uncached) END uncached_tokens,
                   CASE WHEN COUNT(rd.output_total) = COUNT(rd.request_id) AND COUNT(rd.request_id) > 0 THEN SUM(rd.output_total) END output_tokens,
                   CASE WHEN COUNT(rd.output_reasoning) = COUNT(rd.request_id) AND COUNT(rd.request_id) > 0 THEN SUM(rd.output_reasoning) END reasoning_tokens,
                   CASE WHEN COUNT(rd.input_cached) = COUNT(rd.request_id) AND COUNT(rd.input_total) = COUNT(rd.request_id) AND SUM(rd.input_total) > 0 THEN CAST(SUM(rd.input_cached) AS REAL) / SUM(rd.input_total) END cache_ratio,
                   (SELECT estimated_context FROM request_data latest WHERE latest.session_id = s.session_id ORDER BY latest.request_id DESC LIMIT 1) estimated_context_tokens,
                   CASE WHEN SUM(rd.estimated_context) > 0 THEN CAST(SUM(rd.repeated_tokens) AS REAL) / SUM(rd.estimated_context) END repetition,
                   CASE WHEN COUNT(rd.semantic_bp) > 0 THEN AVG(rd.semantic_bp) / 10000.0 END semantic_coverage,
                   CAST((julianday(s.ended_at) - julianday(s.started_at)) * 86400000000 AS INTEGER) duration_us
            FROM sessions s LEFT JOIN request_data rd ON rd.session_id = s.session_id
            GROUP BY s.session_id
        )
        SELECT * FROM aggregate
        WHERE (?1 IS NULL OR model = ?1) AND (?2 IS NULL OR transport = ?2)
          AND (?3 IS NULL OR state = ?3) AND (?4 IS NULL OR has_compaction = ?4)
          AND (?5 IS NULL OR started_at >= ?5) AND (?6 IS NULL OR started_at <= ?6)
          AND (?9 IS NULL OR session_id = ?9)
        ORDER BY {sort_sql} {direction}, session_id DESC LIMIT ?7 OFFSET ?8"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params![
        query.model,
        query.transport,
        query.status,
        query.has_compaction.map(i64::from),
        query.from,
        query.to,
        i64::from(limit) + 1,
        i64::try_from(offset).unwrap_or(i64::MAX),
        query.session_id,
    ])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(session_from_row(row)?);
    }
    let has_more = items.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    if has_more {
        let _removed = items.pop();
    }
    Ok(Page {
        items,
        next_cursor: has_more.then(|| offset.saturating_add(u64::from(limit)).to_string()),
    })
}

fn session_from_row(row: &rusqlite::Row<'_>) -> Result<SessionSummary, rusqlite::Error> {
    let input = row.get::<_, Option<i64>>(8)?.and_then(to_u64);
    let cached = row.get::<_, Option<i64>>(9)?.and_then(to_u64);
    let uncached = row.get::<_, Option<i64>>(10)?.and_then(to_u64).or_else(|| {
        input
            .zip(cached)
            .and_then(|(total, cache)| total.checked_sub(cache))
    });
    let output = row.get::<_, Option<i64>>(11)?.and_then(to_u64);
    let reasoning = row.get::<_, Option<i64>>(12)?.and_then(to_u64);
    let cache_ratio = finite(row.get::<_, Option<f64>>(13)?);
    let estimated = row.get::<_, Option<i64>>(14)?.and_then(to_u64);
    let repetition = finite(row.get::<_, Option<f64>>(15)?);
    let semantic = finite(row.get::<_, Option<f64>>(16)?);
    Ok(SessionSummary {
        id: row.get(0)?,
        workload: None,
        model: row.get(5)?,
        transport: row.get(6)?,
        started_at: row.get(1)?,
        ended_at: row.get(2)?,
        status: row.get(3)?,
        request_count: nonnegative(row.get(4)?),
        has_compaction: row.get::<_, i64>(7)? != 0,
        usage: UsageSummary {
            input_tokens: provider_metric(input),
            cached_input_tokens: provider_metric(cached),
            uncached_input_tokens: provider_metric(uncached),
            cache_ratio: Metric::new(
                cache_ratio,
                source_for(cache_ratio, MetricSource::ProviderReported),
            ),
            output_tokens: provider_metric(output),
            reasoning_tokens: provider_metric(reasoning),
        },
        estimated_context_tokens: estimated_metric(estimated),
        repetition: estimated_ratio(repetition),
        semantic_coverage: estimated_ratio(semantic),
        duration_us: row.get::<_, Option<i64>>(17)?.and_then(to_u64),
        measurement_id: None,
    })
}

pub(crate) fn session_detail(
    connection: &Connection,
    id: &str,
) -> Result<Option<SessionDetail>, rusqlite::Error> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        params![id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let page = sessions(
        connection,
        &SessionsQuery {
            session_id: Some(id.to_owned()),
            ..SessionsQuery::default()
        },
        (1, 0),
    )?;
    let session = page
        .items
        .into_iter()
        .find(|session| session.id == id)
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let requests = session_requests(connection, id, (100, 0))?.items;
    let reasoning_effort = connection.query_row(
        "SELECT pr.reasoning_effort FROM provider_requests pr JOIN operations o ON o.operation_id = pr.operation_id WHERE o.session_id = ?1 AND pr.reasoning_effort IS NOT NULL ORDER BY pr.request_id DESC LIMIT 1",
        params![id],
        |row| row.get(0),
    ).optional()?;
    let context_growth = requests
        .iter()
        .map(|request| ContextGrowthPoint {
            ordinal: request.ordinal,
            provider_input_tokens: request.usage.input_tokens.value,
            cached_input_tokens: request.usage.cached_input_tokens.value,
            uncached_input_tokens: request.usage.uncached_input_tokens.value,
            estimated_context_tokens: request.estimated_context_tokens.value,
        })
        .collect();
    Ok(Some(SessionDetail {
        session,
        reasoning_effort,
        requests,
        context_growth,
    }))
}

pub(crate) fn session_requests(
    connection: &Connection,
    id: &str,
    (limit, offset): (u32, u64),
) -> Result<Page<ProviderRequestSummary>, rusqlite::Error> {
    let sql = format!(
        "SELECT pr.request_id,
                ROW_NUMBER() OVER (ORDER BY pr.request_id), pr.request_kind, pr.model,
                COALESCE(pa.status, pr.observation_status, 'unknown'),
                pu.input_total, pu.input_cached, pu.input_uncached, pu.output_total, pu.output_reasoning,
                pa.duration_us,
                (SELECT SUM(cbo.estimated_tokens) FROM context_snapshots cs JOIN context_block_occurrences cbo ON cbo.snapshot_id = cs.snapshot_id WHERE cs.provider_request_id = pr.request_id ORDER BY cs.analysis_version DESC LIMIT 1),
                (SELECT cam.semantic_coverage_basis_points FROM context_snapshots cs JOIN context_analysis_metrics cam ON cam.snapshot_id = cs.snapshot_id WHERE cs.provider_request_id = pr.request_id ORDER BY cs.analysis_version DESC LIMIT 1)
         FROM provider_requests pr JOIN operations o ON o.operation_id = pr.operation_id
         LEFT JOIN provider_attempts pa ON pa.request_id = pr.request_id AND pa.ordinal = (SELECT MAX(pa2.ordinal) FROM provider_attempts pa2 WHERE pa2.request_id = pr.request_id)
         LEFT JOIN provider_usage pu ON pu.attempt_id = pa.attempt_id
         WHERE o.session_id = ?1 ORDER BY pr.request_id LIMIT ?2 OFFSET ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query(params![
        id,
        i64::from(limit) + 1,
        i64::try_from(offset).unwrap_or(i64::MAX)
    ])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        let input = row.get::<_, Option<i64>>(5)?.and_then(to_u64);
        let cached = row.get::<_, Option<i64>>(6)?.and_then(to_u64);
        let uncached = row.get::<_, Option<i64>>(7)?.and_then(to_u64).or_else(|| {
            input
                .zip(cached)
                .and_then(|(total, cache)| total.checked_sub(cache))
        });
        let cache = input.zip(cached).and_then(|(total, hit)| ratio(hit, total));
        let estimated = row.get::<_, Option<i64>>(11)?.and_then(to_u64);
        let semantic = row
            .get::<_, Option<i64>>(12)?
            .and_then(|value| finite_ratio(value as f64, 10_000.0));
        items.push(ProviderRequestSummary {
            id: row.get(0)?,
            ordinal: nonnegative(row.get(1)?),
            kind: row.get(2)?,
            model: row.get(3)?,
            status: row.get(4)?,
            usage: UsageSummary {
                input_tokens: provider_metric(input),
                cached_input_tokens: provider_metric(cached),
                uncached_input_tokens: provider_metric(uncached),
                cache_ratio: Metric::new(cache, source_for(cache, MetricSource::ProviderReported)),
                output_tokens: provider_metric(row.get::<_, Option<i64>>(8)?.and_then(to_u64)),
                reasoning_tokens: provider_metric(row.get::<_, Option<i64>>(9)?.and_then(to_u64)),
            },
            estimated_context_tokens: estimated_metric(estimated),
            semantic_coverage: estimated_ratio(semantic),
            duration_us: row.get::<_, Option<i64>>(10)?.and_then(to_u64),
        });
    }
    let has_more = items.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    if has_more {
        let _removed = items.pop();
    }
    Ok(Page {
        items,
        next_cursor: has_more.then(|| offset.saturating_add(u64::from(limit)).to_string()),
    })
}

pub(crate) fn session_context(
    connection: &Connection,
    id: &str,
    (limit, offset): (u32, u64),
) -> Result<Option<SessionContext>, rusqlite::Error> {
    let exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE session_id = ?1)",
        params![id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let snapshot: Option<(String, Option<String>)> = connection.query_row(
        "SELECT snapshot_id, logical_context_status FROM context_snapshots WHERE session_id = ?1 ORDER BY provider_request_id DESC, analysis_version DESC LIMIT 1",
        params![id], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let Some((snapshot_id, visibility)) = snapshot else {
        return Ok(Some(SessionContext {
            session_id: id.to_owned(),
            snapshot_id: None,
            visibility: None,
            composition: ContextComposition {
                group_by: "detected_kind".to_owned(),
                categories: Vec::new(),
                estimator_coverage: None,
            },
            blocks: Page {
                items: Vec::new(),
                next_cursor: None,
            },
        }));
    };
    let composition = composition_for_snapshot(connection, &snapshot_id)?;
    let mut statement = connection.prepare(
        "WITH frequencies AS (
            SELECT block_occurrence_id,
                COUNT(*) OVER (PARTITION BY exact_fingerprint) exact_count,
                COUNT(*) OVER (PARTITION BY semantic_fingerprint) semantic_count,
                COUNT(*) OVER (PARTITION BY COALESCE(semantic_fingerprint, exact_fingerprint)) persistence
            FROM context_block_occurrences
        )
        SELECT cbo.block_occurrence_id, cbo.ordinal, cbo.kind, cbo.role, cbo.origin, cbo.detected_kind,
               cbo.raw_bytes, cbo.estimated_tokens,
               CASE WHEN cbo.exact_fingerprint IS NULL THEN NULL ELSE f.exact_count > 1 END,
               CASE WHEN cbo.semantic_fingerprint IS NULL THEN NULL ELSE f.semantic_count > 1 END,
               f.persistence,
               CASE WHEN cbo.exact_fingerprint IS NULL THEN NULL ELSE lower(hex(substr(cbo.exact_fingerprint, 1, 5))) END
        FROM context_block_occurrences cbo JOIN frequencies f USING(block_occurrence_id)
        WHERE cbo.snapshot_id = ?1 ORDER BY cbo.ordinal LIMIT ?2 OFFSET ?3"
    )?;
    let mut rows = statement.query(params![
        snapshot_id,
        i64::from(limit) + 1,
        i64::try_from(offset).unwrap_or(i64::MAX)
    ])?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(ContextBlockSummary {
            id: row.get(0)?,
            ordinal: nonnegative(row.get(1)?),
            kind: row.get(2)?,
            role: row.get(3)?,
            origin: row.get(4)?,
            detected_kind: row.get(5)?,
            raw_bytes: nonnegative(row.get(6)?),
            estimated_tokens: row.get::<_, Option<i64>>(7)?.and_then(to_u64),
            exact_repeated: row.get::<_, Option<i64>>(8)?.map(|value| value != 0),
            semantic_repeated: row.get::<_, Option<i64>>(9)?.map(|value| value != 0),
            persistence: row.get::<_, Option<i64>>(10)?.and_then(to_u64),
            fingerprint_short: row
                .get::<_, Option<String>>(11)?
                .map(|value| format!("{value}…")),
        });
    }
    let has_more = items.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    if has_more {
        let _removed = items.pop();
    }
    Ok(Some(SessionContext {
        session_id: id.to_owned(),
        snapshot_id: Some(snapshot_id),
        visibility,
        composition,
        blocks: Page {
            items,
            next_cursor: has_more.then(|| offset.saturating_add(u64::from(limit)).to_string()),
        },
    }))
}

pub(crate) fn context_explorer(
    connection: &Connection,
    group: &str,
) -> Result<ContextExplorer, rusqlite::Error> {
    let composition = composition(connection, group)?;
    let total: Option<i64> = connection.query_row(
        "SELECT SUM(estimated_tokens) FROM context_block_occurrences",
        [],
        |row| row.get(0),
    )?;
    let mut statement = connection.prepare(
        "SELECT origin, COALESCE(detected_kind, 'unavailable'), COUNT(*), SUM(raw_bytes), SUM(estimated_tokens)
         FROM context_block_occurrences GROUP BY origin, COALESCE(detected_kind, 'unavailable') ORDER BY origin, 5 DESC"
    )?;
    let matrix = statement
        .query_map([], |row| {
            let estimated = row.get::<_, Option<i64>>(4)?.and_then(to_u64);
            Ok(ContextMatrixCell {
                origin: row.get(0)?,
                detected_kind: row.get(1)?,
                block_count: nonnegative(row.get(2)?),
                raw_bytes: nonnegative(row.get::<_, Option<i64>>(3)?.unwrap_or(0)),
                estimated_tokens: estimated,
                estimated_token_share: total
                    .and_then(to_u64)
                    .zip(estimated)
                    .and_then(|(all, value)| ratio(value, all)),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ContextExplorer {
        composition,
        matrix,
    })
}

fn composition(
    connection: &Connection,
    group: &str,
) -> Result<ContextComposition, rusqlite::Error> {
    let column = match group {
        "detected_kind" => "detected_kind",
        "kind" => "kind",
        "origin" => "origin",
        "role" => "role",
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    let sql = format!(
        "WITH frequencies AS (
            SELECT *, COUNT(*) OVER (PARTITION BY exact_fingerprint) exact_count, COUNT(*) OVER (PARTITION BY semantic_fingerprint) semantic_count, COUNT(*) OVER (PARTITION BY COALESCE(semantic_fingerprint, exact_fingerprint)) persistence
            FROM context_block_occurrences
         ), totals AS (SELECT SUM(estimated_tokens) total FROM frequencies)
         SELECT COALESCE({column}, 'unavailable'), COUNT(*), SUM(raw_bytes), SUM(estimated_tokens),
                CASE WHEN (SELECT total FROM totals) > 0 THEN CAST(SUM(estimated_tokens) AS REAL) / (SELECT total FROM totals) END,
                SUM(CASE WHEN exact_fingerprint IS NOT NULL AND exact_count > 1 THEN estimated_tokens END),
                SUM(CASE WHEN semantic_fingerprint IS NOT NULL AND semantic_count > 1 THEN estimated_tokens END),
                AVG(CAST(persistence AS REAL))
         FROM frequencies GROUP BY COALESCE({column}, 'unavailable') ORDER BY SUM(estimated_tokens) DESC"
    );
    let mut statement = connection.prepare(&sql)?;
    let categories = statement
        .query_map([], category_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let coverage = connection.query_row(
        "SELECT CASE WHEN COUNT(*) > 0 THEN CAST(COUNT(estimated_tokens) AS REAL) / COUNT(*) END FROM context_block_occurrences",
        [], |row| row.get::<_, Option<f64>>(0),
    )?;
    Ok(ContextComposition {
        group_by: group.to_owned(),
        categories,
        estimator_coverage: finite(coverage),
    })
}

fn composition_for_snapshot(
    connection: &Connection,
    snapshot: &str,
) -> Result<ContextComposition, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT COALESCE(detected_kind, 'unavailable'), COUNT(*), SUM(raw_bytes), SUM(estimated_tokens),
                CASE WHEN (SELECT SUM(estimated_tokens) FROM context_block_occurrences WHERE snapshot_id = ?1) > 0 THEN CAST(SUM(estimated_tokens) AS REAL) / (SELECT SUM(estimated_tokens) FROM context_block_occurrences WHERE snapshot_id = ?1) END,
                NULL, NULL, NULL
         FROM context_block_occurrences WHERE snapshot_id = ?1 GROUP BY COALESCE(detected_kind, 'unavailable') ORDER BY SUM(estimated_tokens) DESC"
    )?;
    let categories = statement
        .query_map(params![snapshot], category_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let coverage = connection.query_row("SELECT CASE WHEN COUNT(*) > 0 THEN CAST(COUNT(estimated_tokens) AS REAL) / COUNT(*) END FROM context_block_occurrences WHERE snapshot_id = ?1", params![snapshot], |row| row.get::<_, Option<f64>>(0))?;
    Ok(ContextComposition {
        group_by: "detected_kind".to_owned(),
        categories,
        estimator_coverage: finite(coverage),
    })
}

fn category_from_row(row: &rusqlite::Row<'_>) -> Result<ContextCategoryStats, rusqlite::Error> {
    Ok(ContextCategoryStats {
        name: row.get(0)?,
        block_count: nonnegative(row.get(1)?),
        raw_bytes: nonnegative(row.get::<_, Option<i64>>(2)?.unwrap_or(0)),
        estimated_tokens: row.get::<_, Option<i64>>(3)?.and_then(to_u64),
        estimated_token_share: finite(row.get(4)?),
        exact_repeated_tokens: row.get::<_, Option<i64>>(5)?.and_then(to_u64),
        semantic_repeated_tokens: row.get::<_, Option<i64>>(6)?.and_then(to_u64),
        persistence: finite(row.get(7)?),
    })
}

pub(crate) fn unknown(connection: &Connection) -> Result<UnknownSummary, rusqlite::Error> {
    let (blocks, estimated, estimated_blocks, bytes, exact, semantic): (i64, Option<i64>, i64, Option<i64>, i64, i64) = connection.query_row(
        "SELECT COUNT(*), SUM(estimated_tokens), COUNT(estimated_tokens), SUM(raw_bytes), COUNT(DISTINCT exact_fingerprint), COUNT(DISTINCT semantic_fingerprint) FROM context_block_occurrences WHERE COALESCE(detected_kind, 'unknown') = 'unknown'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    let percentile = |fraction: f64| {
        connection.query_row(
        "SELECT raw_bytes FROM context_block_occurrences WHERE COALESCE(detected_kind, 'unknown') = 'unknown' ORDER BY raw_bytes LIMIT 1 OFFSET CAST(MAX((SELECT COUNT(*) FROM context_block_occurrences WHERE COALESCE(detected_kind, 'unknown') = 'unknown') - 1, 0) * ?1 AS INTEGER)",
        params![fraction], |row| row.get::<_, i64>(0),
    ).optional().map(|value| value.and_then(to_u64))
    };
    let persistence = connection.query_row(
        "WITH values_with_frequency AS (SELECT COUNT(*) OVER (PARTITION BY COALESCE(semantic_fingerprint, exact_fingerprint)) frequency FROM context_block_occurrences WHERE COALESCE(detected_kind, 'unknown') = 'unknown') SELECT AVG(CAST(frequency AS REAL)) FROM values_with_frequency",
        [], |row| row.get::<_, Option<f64>>(0),
    )?;
    Ok(UnknownSummary {
        blocks: nonnegative(blocks),
        estimated_tokens: estimated.and_then(to_u64),
        estimator_coverage: ratio(nonnegative(estimated_blocks), nonnegative(blocks)),
        raw_bytes: nonnegative(bytes.unwrap_or(0)),
        unique_exact_fingerprints: nonnegative(exact),
        unique_semantic_fingerprints: nonnegative(semantic),
        size_percentiles: Percentiles {
            p50: percentile(0.50)?,
            p90: percentile(0.90)?,
            p99: percentile(0.99)?,
        },
        persistence: finite(persistence),
    })
}

pub(crate) fn compression_experiments(
    connection: &Connection,
) -> Result<Vec<CompressionExperimentSummary>, rusqlite::Error> {
    if !table_exists(connection, "compression_experiments")? {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT e.experiment_id, e.status, e.runtime_sha, e.started_at, e.completed_at,
                e.forwarding_mutations, e.shadow_drops, e.shadow_queue_full_drops,
                e.shadow_byte_budget_drops, e.shadow_work_budget_drops,
                e.shadow_worker_closed_drops, e.shadow_persistence_drops,
                e.recovery_failures, e.determinism_failures,
                COUNT(c.candidate_id), COUNT(DISTINCT c.block_occurrence_id), COUNT(DISTINCT s.session_id)
         FROM compression_experiments e
         LEFT JOIN compression_candidates c ON c.experiment_id = e.experiment_id
         LEFT JOIN context_snapshots cs ON cs.snapshot_id = c.snapshot_id
         LEFT JOIN sessions s ON s.session_id = cs.session_id
         GROUP BY e.experiment_id
         ORDER BY e.started_at DESC, e.experiment_id DESC",
    )?;
    statement
        .query_map([], |row| {
            Ok(CompressionExperimentSummary {
                id: row.get(0)?,
                status: row.get(1)?,
                runtime_sha: row.get(2)?,
                started_at: row.get(3)?,
                completed_at: row.get(4)?,
                quality: CompressionQuality {
                    forwarding_mutations: nonnegative(row.get(5)?),
                    shadow_drops: nonnegative(row.get(6)?),
                    shadow_queue_full_drops: nonnegative(row.get(7)?),
                    shadow_byte_budget_drops: nonnegative(row.get(8)?),
                    shadow_work_budget_drops: nonnegative(row.get(9)?),
                    shadow_worker_closed_drops: nonnegative(row.get(10)?),
                    shadow_persistence_drops: nonnegative(row.get(11)?),
                    recovery_failures: nonnegative(row.get(12)?),
                    determinism_failures: nonnegative(row.get(13)?),
                },
                candidate_count: nonnegative(row.get(14)?),
                block_count: nonnegative(row.get(15)?),
                session_count: nonnegative(row.get(16)?),
            })
        })?
        .collect()
}

pub(crate) fn compression_experiment(
    connection: &Connection,
    experiment_id: &str,
) -> Result<Option<CompressionExperimentDetail>, rusqlite::Error> {
    let Some(summary) = compression_experiments(connection)?
        .into_iter()
        .find(|experiment| experiment.id == experiment_id)
    else {
        return Ok(None);
    };
    let (compressor_set_json, limits_json) = connection.query_row(
        "SELECT compressor_set_json, limits_json FROM compression_experiments WHERE experiment_id = ?1",
        [experiment_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let compressors = compressor_summaries(connection, experiment_id)?;
    Ok(Some(CompressionExperimentDetail {
        summary,
        compressor_set_json,
        limits_json,
        compressors,
        reduction_histogram: compression_histogram(connection, experiment_id, false)?,
        latency_histogram: compression_histogram(connection, experiment_id, true)?,
        workload_distribution_available: false,
    }))
}

fn compressor_summaries(
    connection: &Connection,
    experiment_id: &str,
) -> Result<Vec<CompressorSummary>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT c.compressor_id, c.compressor_version, COUNT(*),
                SUM(c.status = 'applicable'),
                SUM(CASE WHEN c.status = 'applicable' THEN m.input_bytes END),
                SUM(CASE WHEN c.status = 'applicable' THEN m.output_bytes END),
                SUM(CASE WHEN c.status = 'applicable' THEN m.bytes_delta END),
                SUM(CASE WHEN c.status = 'applicable' THEN m.input_estimated_tokens END),
                SUM(CASE WHEN c.status = 'applicable' THEN m.output_estimated_tokens END),
                SUM(CASE WHEN c.status = 'applicable' THEN m.estimated_token_delta END),
                SUM(CASE WHEN m.reversible = 1 AND m.output_bytes IS NOT NULL
                         THEN m.recovery_verified END),
                SUM(m.reversible = 1 AND m.output_bytes IS NOT NULL),
                SUM(CASE WHEN m.output_bytes IS NOT NULL THEN m.deterministic END),
                SUM(m.output_bytes IS NOT NULL),
                SUM(c.cache_risk = 'low'), SUM(c.cache_risk = 'medium'),
                SUM(c.cache_risk = 'high'), SUM(c.cache_risk = 'unknown')
         FROM compression_candidates c
         JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id
         WHERE c.experiment_id = ?1
         GROUP BY c.compressor_id, c.compressor_version
         ORDER BY c.compressor_id",
    )?;
    let rows = statement.query_map([experiment_id], |row| {
        let eligible = nonnegative(row.get(2)?);
        let applicable = nonnegative(row.get(3)?);
        let input_bytes = row.get::<_, Option<i64>>(4)?.and_then(to_u64);
        let output_bytes = row.get::<_, Option<i64>>(5)?.and_then(to_u64);
        let byte_reduction = row.get::<_, Option<i64>>(6)?.and_then(to_u64);
        let estimated_input_tokens = row.get::<_, Option<i64>>(7)?.and_then(to_u64);
        let estimated_output_tokens = row.get::<_, Option<i64>>(8)?.and_then(to_u64);
        let estimated_reduction = row.get::<_, Option<i64>>(9)?.and_then(to_u64);
        Ok((
            CompressorSummary {
                compressor: row.get(0)?,
                version: row.get(1)?,
                eligible_blocks: eligible,
                applicable_blocks: applicable,
                applicability_basis_points: ratio_basis_points(applicable, eligible),
                input_bytes,
                output_bytes,
                byte_reduction,
                candidate_effective_byte_reduction: byte_reduction,
                unique_candidate_byte_reduction: None,
                byte_reduction_basis_points: input_bytes
                    .zip(byte_reduction)
                    .and_then(|(total, reduction)| ratio_basis_points(reduction, total)),
                estimated_input_tokens,
                estimated_output_tokens,
                estimated_reduction,
                candidate_effective_estimated_token_reduction: estimated_reduction,
                unique_candidate_estimated_token_reduction: None,
                estimated_reduction_basis_points: estimated_input_tokens
                    .zip(estimated_reduction)
                    .and_then(|(total, reduction)| ratio_basis_points(reduction, total)),
                recovery_basis_points: ratio_basis_points(
                    nonnegative(row.get::<_, Option<i64>>(10)?.unwrap_or(0)),
                    nonnegative(row.get(11)?),
                ),
                deterministic_basis_points: ratio_basis_points(
                    nonnegative(row.get::<_, Option<i64>>(12)?.unwrap_or(0)),
                    nonnegative(row.get(13)?),
                ),
                processing_p50_us: None,
                processing_p90_us: None,
                processing_p95_us: None,
                processing_p99_us: None,
                cache_risk_low: nonnegative(row.get(14)?),
                cache_risk_medium: nonnegative(row.get(15)?),
                cache_risk_high: nonnegative(row.get(16)?),
                cache_risk_unknown: nonnegative(row.get(17)?),
            },
            eligible,
        ))
    })?;
    let mut summaries = Vec::new();
    for row in rows {
        let (mut summary, count) = row?;
        summary.processing_p50_us =
            compressor_percentile(connection, experiment_id, &summary.compressor, count, 50)?;
        summary.processing_p90_us =
            compressor_percentile(connection, experiment_id, &summary.compressor, count, 90)?;
        summary.processing_p95_us =
            compressor_percentile(connection, experiment_id, &summary.compressor, count, 95)?;
        summary.processing_p99_us =
            compressor_percentile(connection, experiment_id, &summary.compressor, count, 99)?;
        let (unique_bytes, unique_tokens) =
            unique_candidate_reductions(connection, experiment_id, &summary.compressor)?;
        summary.unique_candidate_byte_reduction = unique_bytes;
        summary.unique_candidate_estimated_token_reduction = unique_tokens;
        summaries.push(summary);
    }
    Ok(summaries)
}

fn unique_candidate_reductions(
    connection: &Connection,
    experiment_id: &str,
    compressor: &str,
) -> Result<(Option<u64>, Option<u64>), rusqlite::Error> {
    connection.query_row(
        "SELECT SUM(bytes_delta), SUM(estimated_token_delta)
         FROM (
             SELECT c.original_fingerprint,
                    MAX(m.bytes_delta) AS bytes_delta,
                    MAX(m.estimated_token_delta) AS estimated_token_delta
             FROM compression_candidates c
             JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id
             WHERE c.experiment_id = ?1 AND c.compressor_id = ?2
               AND c.status = 'applicable'
             GROUP BY c.original_fingerprint
         )",
        params![experiment_id, compressor],
        |row| {
            Ok((
                row.get::<_, Option<i64>>(0)?.and_then(to_u64),
                row.get::<_, Option<i64>>(1)?.and_then(to_u64),
            ))
        },
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "bounded percentile query keeps scope explicit"
)]
fn compressor_percentile(
    connection: &Connection,
    experiment_id: &str,
    compressor: &str,
    count: u64,
    percentile: u64,
) -> Result<Option<u64>, rusqlite::Error> {
    if count == 0 {
        return Ok(None);
    }
    let offset = count.saturating_sub(1).saturating_mul(percentile) / 100;
    connection
        .query_row(
            "SELECT m.processing_us FROM compression_candidates c JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id WHERE c.experiment_id = ?1 AND c.compressor_id = ?2 ORDER BY m.processing_us LIMIT 1 OFFSET ?3",
            params![experiment_id, compressor, i64::try_from(offset).unwrap_or(i64::MAX)],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|value| value.and_then(to_u64))
}

fn compression_histogram(
    connection: &Connection,
    experiment_id: &str,
    latency: bool,
) -> Result<Vec<CompressionHistogramBucket>, rusqlite::Error> {
    let buckets: &[(&str, u64, Option<u64>)] = if latency {
        &[
            ("<100 us", 0, Some(100)),
            ("100-499 us", 100, Some(500)),
            ("500-999 us", 500, Some(1_000)),
            ("1-5 ms", 1_000, Some(5_000)),
            (">=5 ms", 5_000, None),
        ]
    } else {
        &[
            ("0-4%", 0, Some(500)),
            ("5-14%", 500, Some(1_500)),
            ("15-29%", 1_500, Some(3_000)),
            ("30-49%", 3_000, Some(5_000)),
            (">=50%", 5_000, None),
        ]
    };
    let mut result = Vec::new();
    for (label, lower, upper) in buckets {
        let (expression, status_filter) = if latency {
            ("m.processing_us", "")
        } else {
            (
                "CASE WHEN m.input_bytes > 0 THEN m.bytes_delta * 10000 / m.input_bytes END",
                "AND c.status = 'applicable'",
            )
        };
        let upper_filter =
            upper.map_or_else(String::new, |value| format!("AND {expression} < {value}"));
        let sql = format!(
            "SELECT COUNT(*) FROM compression_candidates c JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id WHERE c.experiment_id = ?1 {status_filter} AND {expression} >= ?2 {upper_filter}"
        );
        let count = connection.query_row(
            &sql,
            params![experiment_id, i64::try_from(*lower).unwrap_or(i64::MAX)],
            |row| row.get::<_, i64>(0),
        )?;
        result.push(CompressionHistogramBucket {
            label: (*label).to_owned(),
            lower_inclusive: *lower,
            upper_exclusive: *upper,
            count: nonnegative(count),
        });
    }
    Ok(result)
}

#[allow(
    clippy::too_many_arguments,
    reason = "bounded page query keeps experiment scope explicit"
)]
pub(crate) fn compression_candidates(
    connection: &Connection,
    experiment_id: &str,
    limit: u32,
    offset: u64,
) -> Result<Page<CompressionCandidateSummary>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT c.candidate_id, c.experiment_id, c.snapshot_id, b.ordinal, b.kind, b.origin,
                b.detected_kind, c.compressor_id, c.compressor_version, c.status,
                m.input_bytes, m.output_bytes, m.bytes_delta, m.input_estimated_tokens,
                m.output_estimated_tokens, m.estimated_token_delta, m.recovery_verified,
                m.deterministic, m.processing_us, c.preserved_prefix_bytes,
                m.preserved_prefix_ratio_basis_points, c.cache_risk,
                CASE WHEN b.exact_fingerprint IS NULL THEN NULL ELSE
                    (SELECT COUNT(*) > 1 FROM context_block_occurrences bx WHERE bx.exact_fingerprint = b.exact_fingerprint) END,
                CASE WHEN b.exact_fingerprint IS NULL THEN NULL ELSE
                    (SELECT COUNT(*) FROM context_block_occurrences bx WHERE bx.exact_fingerprint = b.exact_fingerprint) END
         FROM compression_candidates c
         JOIN compression_candidate_metrics m ON m.candidate_id = c.candidate_id
         JOIN context_block_occurrences b ON b.block_occurrence_id = c.block_occurrence_id
         WHERE c.experiment_id = ?1
         ORDER BY c.candidate_id
         LIMIT ?2 OFFSET ?3",
    )?;
    let requested = u64::from(limit).saturating_add(1);
    let rows = statement.query_map(
        params![
            experiment_id,
            i64::try_from(requested).unwrap_or(i64::MAX),
            i64::try_from(offset).unwrap_or(i64::MAX)
        ],
        |row| {
            Ok(CompressionCandidateSummary {
                id: row.get(0)?,
                experiment_id: row.get(1)?,
                snapshot_id: row.get(2)?,
                block_ordinal: nonnegative(row.get(3)?),
                block_kind: row.get(4)?,
                origin: row.get(5)?,
                detected_kind: row.get(6)?,
                compressor: row.get(7)?,
                version: row.get(8)?,
                status: row.get(9)?,
                input_bytes: nonnegative(row.get(10)?),
                output_bytes: row.get::<_, Option<i64>>(11)?.and_then(to_u64),
                byte_reduction: row.get::<_, Option<i64>>(12)?.and_then(to_u64),
                input_estimated_tokens: row.get::<_, Option<i64>>(13)?.and_then(to_u64),
                output_estimated_tokens: row.get::<_, Option<i64>>(14)?.and_then(to_u64),
                estimated_reduction: row.get::<_, Option<i64>>(15)?.and_then(to_u64),
                recovery_verified: row.get(16)?,
                deterministic: row.get(17)?,
                latency_us: row.get::<_, Option<i64>>(18)?.and_then(to_u64),
                preserved_prefix_bytes: row.get::<_, Option<i64>>(19)?.and_then(to_u64),
                preserved_prefix_ratio_basis_points: row
                    .get::<_, Option<i64>>(20)?
                    .and_then(to_u64)
                    .and_then(|value| u16::try_from(value).ok()),
                cache_risk: row.get(21)?,
                exact_repetition: row.get(22)?,
                persistence: row.get::<_, Option<i64>>(23)?.and_then(to_u64),
            })
        },
    )?;
    let mut items = rows.collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > usize::try_from(limit).unwrap_or(usize::MAX);
    items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
    Ok(Page {
        items,
        next_cursor: has_more.then(|| offset.saturating_add(u64::from(limit)).to_string()),
    })
}

fn table_exists(connection: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn ratio_basis_points(numerator: u64, denominator: u64) -> Option<u16> {
    if denominator == 0 {
        return None;
    }
    let value = numerator.saturating_mul(10_000) / denominator;
    u16::try_from(value.min(10_000)).ok()
}

fn provider_metric(value: Option<u64>) -> Metric<u64> {
    Metric::new(value, source_for(value, MetricSource::ProviderReported))
}

fn estimated_metric(value: Option<u64>) -> Metric<u64> {
    Metric::new(value, source_for(value, MetricSource::LocallyEstimated))
}

fn estimated_ratio(value: Option<f64>) -> Metric<f64> {
    Metric::new(value, source_for(value, MetricSource::LocallyEstimated))
}

fn source_for<T>(value: Option<T>, source: MetricSource) -> MetricSource {
    if value.is_some() {
        source
    } else {
        MetricSource::Unavailable
    }
}

fn to_u64(value: i64) -> Option<u64> {
    u64::try_from(value).ok()
}

fn nonnegative(value: i64) -> u64 {
    to_u64(value).unwrap_or(0)
}

fn ratio(numerator: u64, denominator: u64) -> Option<f64> {
    if denominator == 0 {
        None
    } else {
        finite_ratio(numerator as f64, denominator as f64)
    }
}

fn finite_ratio(numerator: f64, denominator: f64) -> Option<f64> {
    let value = numerator / denominator;
    value.is_finite().then_some(value)
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite())
}

#[cfg(test)]
mod tests {
    use super::open_read_only;
    use crate::fixture_database;

    #[test]
    fn operational_connection_rejects_writes_at_sqlite_level() {
        let fixture = fixture_database(false).expect("fixture");
        let connection = open_read_only(fixture.path()).expect("read-only connection");
        let result = connection.execute(
            "INSERT INTO sessions(session_id, started_at, state, ingress_key) VALUES ('forbidden', 'now', 'running', 'forbidden')",
            [],
        );
        assert!(result.is_err());
        let query_only: bool = connection
            .pragma_query_value(None, "query_only", |row| row.get(0))
            .expect("query_only pragma");
        assert!(query_only);
    }

    #[test]
    fn detail_query_plans_use_existing_schema_indexes() {
        let fixture = fixture_database(false).expect("fixture");
        let connection = open_read_only(fixture.path()).expect("read-only connection");
        let explain = |sql: &str, parameter: &str| {
            let mut statement = connection.prepare(sql).expect("prepare explain");
            statement
                .query_map([parameter], |row| row.get::<_, String>(3))
                .expect("query plan")
                .collect::<Result<Vec<_>, _>>()
                .expect("plan rows")
                .join("\n")
        };
        let session = explain(
            "EXPLAIN QUERY PLAN SELECT * FROM sessions WHERE session_id = ?1",
            "00000000-0000-7000-8000-000000000000",
        );
        let snapshots = explain(
            "EXPLAIN QUERY PLAN SELECT snapshot_id FROM context_snapshots WHERE session_id = ?1 ORDER BY snapshot_id LIMIT 1",
            "00000000-0000-7000-8000-000000000000",
        );
        let blocks = explain(
            "EXPLAIN QUERY PLAN SELECT block_occurrence_id FROM context_block_occurrences WHERE snapshot_id = ?1 ORDER BY ordinal LIMIT 100",
            "snapshot-0000-00",
        );
        assert!(session.contains("SEARCH sessions"), "{session}");
        assert!(
            snapshots.contains("context_snapshots_session_idx"),
            "{snapshots}"
        );
        assert!(
            blocks.contains("sqlite_autoindex_context_block_occurrences_2"),
            "{blocks}"
        );
    }
}
