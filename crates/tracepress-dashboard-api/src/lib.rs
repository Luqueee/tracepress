//! Read-only, versioned HTTP API for Tracepress Observatory.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::exhaustive_enums,
    clippy::exhaustive_structs,
    clippy::expect_used,
    clippy::missing_const_for_fn,
    clippy::missing_errors_doc,
    clippy::multiple_crate_versions,
    clippy::needless_pass_by_value,
    clippy::rc_buffer,
    clippy::redundant_pub_crate,
    clippy::too_many_lines,
    clippy::trivially_copy_pass_by_ref,
    clippy::uninlined_format_args,
    clippy::useless_format,
    reason = "the bounded HTTP/SQLite projection boundary favors explicit wire code and synthetic test readability"
)]

mod baseline;
mod db;
mod fixture;

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use thiserror::Error;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::{
    catch_panic::CatchPanicLayer, set_header::SetResponseHeaderLayer, timeout::TimeoutLayer,
};
use tracepress_dashboard_types::{
    ApiError, ApiErrorResponse, BaselineDetail, BaselineSummary, CompressionCandidateSummary,
    CompressionExperimentDetail, CompressionExperimentSummary, ContextExplorer, OpportunitySummary,
    Overview, Page, ProviderRequestSummary, SessionContext, SessionDetail, SessionSummary,
    UnknownSummary, WorkloadSummary,
};

/// Default local-only Observatory port.
pub const DEFAULT_PORT: u16 = 4319;
const MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_PAGE_SIZE: u32 = 50;
// Keep every request bounded while allowing the documented large-dataset smoke
// (1,000 sessions / 10,000 requests) to complete under workspace contention.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Dashboard server configuration.
#[derive(Clone, Debug)]
pub struct DashboardConfig {
    /// Operational SQLite path. It is always opened read-only.
    pub database_path: PathBuf,
    /// Repository report root used only for baseline artifact views.
    pub reports_path: PathBuf,
    /// Bind address. Non-loopback addresses are rejected in Phase 4.0.
    pub bind: SocketAddr,
    /// Optional compiled Dioxus public directory.
    pub assets_path: Option<PathBuf>,
}

impl DashboardConfig {
    /// Builds the secure local default configuration.
    #[must_use]
    pub fn local(database_path: PathBuf, reports_path: PathBuf) -> Self {
        Self {
            database_path,
            reports_path,
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT),
            assets_path: None,
        }
    }
}

#[derive(Clone, Debug)]
struct AppState {
    database_path: Arc<PathBuf>,
    reports_path: Arc<PathBuf>,
}

/// Errors returned by dashboard startup.
#[derive(Debug, Error)]
pub enum ServeError {
    /// Phase 4.0 refuses remote binds by design.
    #[error("Phase 4.0 only permits loopback bind addresses")]
    NonLoopbackBind,
    /// Database validation failed.
    #[error("dashboard database is unavailable: {0}")]
    Database(String),
    /// Listener or server failure.
    #[error("dashboard server failed: {0}")]
    Server(String),
}

#[derive(Debug)]
enum ApiFailure {
    NotFound(&'static str, &'static str),
    Invalid(&'static str),
    Internal,
}

impl IntoResponse for ApiFailure {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::NotFound(code, message) => (StatusCode::NOT_FOUND, code, message),
            Self::Invalid(message) => (StatusCode::BAD_REQUEST, "invalid_query", message),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "dashboard_query_failed",
                "The dashboard could not complete this read-only query",
            ),
        };
        (
            status,
            Json(ApiErrorResponse {
                error: ApiError {
                    code: code.to_owned(),
                    message: message.to_owned(),
                },
            }),
        )
            .into_response()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PageQuery {
    limit: Option<u32>,
    cursor: Option<String>,
}

impl PageQuery {
    fn bounds(&self) -> Result<(u32, u64), ApiFailure> {
        let limit = self.limit.unwrap_or(DEFAULT_PAGE_SIZE);
        if limit == 0 || limit > MAX_PAGE_SIZE {
            return Err(ApiFailure::Invalid("limit must be between 1 and 100"));
        }
        let offset = self
            .cursor
            .as_deref()
            .unwrap_or("0")
            .parse::<u64>()
            .map_err(|_error| ApiFailure::Invalid("cursor is invalid"))?;
        Ok((limit, offset))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct SessionsQuery {
    #[serde(skip)]
    session_id: Option<String>,
    limit: Option<u32>,
    cursor: Option<String>,
    workload: Option<String>,
    model: Option<String>,
    transport: Option<String>,
    status: Option<String>,
    has_compaction: Option<bool>,
    measurement_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
    sort: Option<String>,
    direction: Option<String>,
}

impl SessionsQuery {
    fn page(&self) -> PageQuery {
        PageQuery {
            limit: self.limit,
            cursor: self.cursor.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct ContextQuery {
    group_by: Option<String>,
}

/// Builds the versioned API router with browser security headers.
pub fn router(database_path: PathBuf, reports_path: PathBuf) -> Router {
    let state = AppState {
        database_path: Arc::new(database_path),
        reports_path: Arc::new(reports_path),
    };
    let api = Router::new()
        .route("/overview", get(overview))
        .route("/sessions", get(sessions))
        .route("/sessions/{id}", get(session_detail))
        .route("/sessions/{id}/requests", get(session_requests))
        .route("/sessions/{id}/context", get(session_context))
        .route("/context/composition", get(context_composition))
        .route("/context/repetition", get(context_repetition))
        .route("/context/unknown", get(unknown))
        .route("/workloads", get(workloads))
        .route("/baselines", get(baselines))
        .route("/baselines/{id}", get(baseline_detail))
        .route("/opportunities", get(opportunities))
        .route("/compression/experiments", get(compression_experiments))
        .route("/compression/experiments/{id}", get(compression_experiment))
        .route(
            "/compression/experiments/{id}/candidates",
            get(compression_candidates),
        )
        .route("/health", get(health));
    Router::new()
        .nest("/api/v1", api)
        .with_state(state)
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ))
        .layer(CatchPanicLayer::new())
}

/// Validates the read-only database and serves until shutdown.
pub async fn serve(config: DashboardConfig) -> Result<(), ServeError> {
    if !config.bind.ip().is_loopback() {
        return Err(ServeError::NonLoopbackBind);
    }
    db::validate_database(&config.database_path)
        .map_err(|error| ServeError::Database(error.to_string()))?;
    let listener = tokio::net::TcpListener::bind(config.bind)
        .await
        .map_err(|error| ServeError::Server(error.to_string()))?;
    let mut application = router(config.database_path, config.reports_path);
    if let Some(assets) = config
        .assets_path
        .filter(|path| path.join("index.html").is_file())
    {
        let index = assets.join("index.html");
        application =
            application.fallback_service(ServeDir::new(assets).fallback(ServeFile::new(index)));
    } else {
        application = application.fallback(get(frontend_unavailable));
    }
    application = application
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; style-src-attr 'unsafe-inline'; script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
            ),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("no-referrer"),
        ));
    axum::serve(listener, application)
        .await
        .map_err(|error| ServeError::Server(error.to_string()))
}

async fn frontend_unavailable() -> impl IntoResponse {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        "<!doctype html><html><head><title>Tracepress Observatory</title></head><body><main><h1>Tracepress Observatory</h1><p>The API is healthy, but compiled Dioxus assets were not found. Run <code>dx build --release</code> in crates/tracepress-dashboard.</p></main></body></html>",
    )
}

/// Creates a synthetic fixture database in a temporary directory.
pub fn fixture_database(large: bool) -> Result<FixtureDatabase, ServeError> {
    fixture::create(large).map_err(|error| ServeError::Database(error.to_string()))
}

/// Temporary synthetic database which is deleted when dropped.
#[derive(Debug)]
pub struct FixtureDatabase {
    _directory: tempfile::TempDir,
    path: PathBuf,
}

impl FixtureDatabase {
    /// Returns the fixture SQLite path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn new(directory: tempfile::TempDir, path: PathBuf) -> Self {
        Self {
            _directory: directory,
            path,
        }
    }
}

async fn overview(State(state): State<AppState>) -> Result<Json<Overview>, ApiFailure> {
    db::run(state.database_path, db::overview).await.map(Json)
}

async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> Result<Json<Page<SessionSummary>>, ApiFailure> {
    let bounds = query.page().bounds()?;
    db::run(state.database_path, move |connection| {
        db::sessions(connection, &query, bounds)
    })
    .await
    .map(Json)
}

async fn session_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<SessionDetail>, ApiFailure> {
    db::run(state.database_path, move |connection| {
        db::session_detail(connection, &id)
    })
    .await
    .and_then(|value| {
        value.map(Json).ok_or(ApiFailure::NotFound(
            "session_not_found",
            "Session was not found",
        ))
    })
}

async fn session_requests(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<ProviderRequestSummary>>, ApiFailure> {
    let bounds = page.bounds()?;
    db::run(state.database_path, move |connection| {
        db::session_requests(connection, &id, bounds)
    })
    .await
    .map(Json)
}

async fn session_context(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(page): Query<PageQuery>,
) -> Result<Json<SessionContext>, ApiFailure> {
    let bounds = page.bounds()?;
    db::run(state.database_path, move |connection| {
        db::session_context(connection, &id, bounds)
    })
    .await
    .and_then(|value| {
        value.map(Json).ok_or(ApiFailure::NotFound(
            "session_not_found",
            "Session was not found",
        ))
    })
}

async fn context_composition(
    State(state): State<AppState>,
    Query(query): Query<ContextQuery>,
) -> Result<Json<ContextExplorer>, ApiFailure> {
    let group = query.group_by.unwrap_or_else(|| "detected_kind".to_owned());
    if !matches!(group.as_str(), "detected_kind" | "kind" | "origin" | "role") {
        return Err(ApiFailure::Invalid("unsupported context grouping"));
    }
    db::run(state.database_path, move |connection| {
        db::context_explorer(connection, &group)
    })
    .await
    .map(Json)
}

async fn context_repetition(
    State(state): State<AppState>,
) -> Result<Json<tracepress_dashboard_types::RepetitionSummary>, ApiFailure> {
    db::run(state.database_path, db::repetition).await.map(Json)
}

async fn unknown(State(state): State<AppState>) -> Result<Json<UnknownSummary>, ApiFailure> {
    db::run(state.database_path, db::unknown).await.map(Json)
}

async fn workloads(
    State(state): State<AppState>,
) -> Result<Json<Vec<WorkloadSummary>>, ApiFailure> {
    baseline::load_workloads(&state.reports_path)
        .map(Json)
        .map_err(|_error| ApiFailure::Internal)
}

async fn baselines(
    State(state): State<AppState>,
) -> Result<Json<Vec<BaselineSummary>>, ApiFailure> {
    baseline::list(&state.reports_path)
        .map(Json)
        .map_err(|_error| ApiFailure::Internal)
}

async fn baseline_detail(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<BaselineDetail>, ApiFailure> {
    if !is_safe_baseline_id(&id) {
        return Err(ApiFailure::Invalid("baseline id is invalid"));
    }
    baseline::detail(&state.reports_path, &id)
        .map_err(|_error| ApiFailure::Internal)?
        .map(Json)
        .ok_or(ApiFailure::NotFound(
            "baseline_not_found",
            "Baseline was not found",
        ))
}

async fn opportunities(
    State(state): State<AppState>,
) -> Result<Json<Vec<OpportunitySummary>>, ApiFailure> {
    baseline::load_opportunities(&state.reports_path)
        .map(Json)
        .map_err(|_error| ApiFailure::Internal)
}

async fn compression_experiments(
    State(state): State<AppState>,
) -> Result<Json<Vec<CompressionExperimentSummary>>, ApiFailure> {
    db::run(state.database_path, db::compression_experiments)
        .await
        .map(Json)
}

async fn compression_experiment(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<Json<CompressionExperimentDetail>, ApiFailure> {
    if !is_safe_experiment_id(&id) {
        return Err(ApiFailure::Invalid("experiment id is invalid"));
    }
    db::run(state.database_path, move |connection| {
        db::compression_experiment(connection, &id)
    })
    .await?
    .map(Json)
    .ok_or(ApiFailure::NotFound(
        "compression_experiment_not_found",
        "Compression experiment was not found",
    ))
}

async fn compression_candidates(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Page<CompressionCandidateSummary>>, ApiFailure> {
    if !is_safe_experiment_id(&id) {
        return Err(ApiFailure::Invalid("experiment id is invalid"));
    }
    let (limit, offset) = query.bounds()?;
    db::run(state.database_path, move |connection| {
        db::compression_candidates(connection, &id, limit, offset)
    })
    .await
    .map(Json)
}

async fn health(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiFailure> {
    db::run(state.database_path, db::health).await?;
    Ok(Json(serde_json::json!({"ok": true, "mode": "read_only"})))
}

fn is_safe_baseline_id(id: &str) -> bool {
    id.len() <= 64
        && !id.is_empty()
        && id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

fn is_safe_experiment_id(id: &str) -> bool {
    id.len() <= 128
        && !id.is_empty()
        && id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

impl FromStr for SessionsSort {
    type Err = ApiFailure;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "created_at" => Ok(Self::CreatedAt),
            "request_count" => Ok(Self::RequestCount),
            "input_tokens" => Ok(Self::InputTokens),
            "cache_ratio" => Ok(Self::CacheRatio),
            "estimated_context_tokens" => Ok(Self::EstimatedContextTokens),
            "repetition" => Ok(Self::Repetition),
            _ => Err(ApiFailure::Invalid("unsupported session sort")),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum SessionsSort {
    CreatedAt,
    RequestCount,
    InputTokens,
    CacheRatio,
    EstimatedContextTokens,
    Repetition,
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;
    use tracepress_dashboard_types::{
        CompressionCandidateSummary, CompressionExperimentDetail, CompressionExperimentSummary,
        Overview, Page, SessionSummary,
    };

    use super::{db, fixture_database, router};

    async fn response(path: &str, large: bool) -> (StatusCode, Vec<u8>) {
        let fixture = fixture_database(large).expect("create synthetic fixture");
        let reports = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reports");
        let response = router(fixture.path().to_path_buf(), reports)
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes()
            .to_vec();
        (status, bytes)
    }

    #[tokio::test]
    async fn csp_allows_only_runtime_style_attributes() {
        let fixture = fixture_database(false).expect("create synthetic fixture");
        let reports = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reports");
        let response = router(fixture.path().to_path_buf(), reports)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/health")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let policy = response
            .headers()
            .get("content-security-policy")
            .and_then(|value| value.to_str().ok())
            .expect("CSP header");
        assert!(policy.contains("style-src 'self'"));
        assert!(policy.contains("style-src-attr 'unsafe-inline'"));
        assert!(!policy.contains("style-src 'unsafe-inline'"));
    }

    #[tokio::test]
    async fn overview_projects_usage_and_degraded_quality() {
        let (status, body) = response("/api/v1/overview", false).await;
        assert_eq!(status, StatusCode::OK);
        let overview: Overview = serde_json::from_slice(&body).expect("overview JSON");
        assert_eq!(overview.sessions, 4);
        assert_eq!(overview.provider_requests, 16);
        assert_eq!(overview.quality.status, "degraded");
        assert_eq!(overview.usage.input_tokens.value, Some(24_400));
        let text = String::from_utf8(body).expect("UTF-8 JSON");
        for forbidden in ["authorization", "cookie", "raw_usage_json", "request_body"] {
            assert!(!text.contains(forbidden));
        }
    }

    #[tokio::test]
    async fn sessions_filter_and_cursor_pagination_are_bounded() {
        let (status, body) = response(
            "/api/v1/sessions?limit=1&model=gpt-5&sort=input_tokens",
            false,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let page: Page<SessionSummary> = serde_json::from_slice(&body).expect("sessions JSON");
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.items.first().and_then(|item| item.model.as_deref()),
            Some("gpt-5")
        );
        assert_eq!(page.next_cursor.as_deref(), Some("1"));

        let (invalid_status, _) = response("/api/v1/sessions?limit=101", false).await;
        assert_eq!(invalid_status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn session_detail_and_not_found_are_typed() {
        let id = "00000000-0000-7000-8000-000000000001";
        let (status, body) = response(&format!("/api/v1/sessions/{id}"), false).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let detail: tracepress_dashboard_types::SessionDetail =
            serde_json::from_slice(&body).expect("session detail JSON");
        assert_eq!(detail.requests.len(), 4);
        assert!(
            detail
                .requests
                .iter()
                .any(|request| request.kind == "compaction_v2")
        );

        let (missing_status, missing_body) = response("/api/v1/sessions/missing", false).await;
        assert_eq!(missing_status, StatusCode::NOT_FOUND);
        let message = String::from_utf8(missing_body).expect("UTF-8 error");
        assert!(message.contains("session_not_found"));
        assert!(!message.contains("SELECT"));
    }

    #[tokio::test]
    async fn large_fixture_paginates_without_returning_all_rows() {
        let (status, body) = response("/api/v1/sessions?limit=50", true).await;
        assert_eq!(status, StatusCode::OK);
        let page: Page<SessionSummary> =
            serde_json::from_slice(&body).expect("large sessions JSON");
        assert_eq!(page.items.len(), 50);
        assert_eq!(page.next_cursor.as_deref(), Some("50"));
    }

    #[tokio::test]
    async fn compression_experiment_and_candidate_pages_are_real_and_bounded() {
        let (status, body) = response("/api/v1/compression/experiments", false).await;
        assert_eq!(status, StatusCode::OK);
        let experiments: Vec<CompressionExperimentSummary> =
            serde_json::from_slice(&body).expect("compression experiments JSON");
        assert_eq!(experiments.len(), 1);
        assert_eq!(
            experiments
                .first()
                .expect("one experiment")
                .quality
                .forwarding_mutations,
            0
        );
        assert_eq!(
            experiments.first().expect("one experiment").candidate_count,
            144
        );
        assert_eq!(experiments.first().expect("one experiment").block_count, 16);
        assert_eq!(
            experiments.first().expect("one experiment").session_count,
            4
        );

        let (status, body) =
            response("/api/v1/compression/experiments/shadow-pilot-001", false).await;
        assert_eq!(status, StatusCode::OK);
        let detail: CompressionExperimentDetail =
            serde_json::from_slice(&body).expect("compression detail JSON");
        assert_eq!(detail.compressors.len(), 9);
        assert!(!detail.workload_distribution_available);
        let minify = detail
            .compressors
            .iter()
            .find(|summary| summary.compressor == "json.minify")
            .expect("json.minify summary");
        assert!(minify.candidate_effective_byte_reduction.is_some());
        assert!(minify.unique_candidate_byte_reduction.is_some());
        assert!(minify.unique_candidate_byte_reduction < minify.candidate_effective_byte_reduction);
        assert_eq!(minify.recovery_basis_points, Some(10_000));
        assert_eq!(minify.deterministic_basis_points, Some(10_000));
        let tabular = detail
            .compressors
            .iter()
            .find(|summary| summary.compressor == "json.tabular")
            .expect("json.tabular summary");
        assert_eq!(tabular.recovery_basis_points, Some(10_000));
        assert_eq!(tabular.deterministic_basis_points, Some(10_000));
        let (status, body) = response(
            "/api/v1/compression/experiments/shadow-pilot-001/candidates?limit=2",
            false,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let page: Page<CompressionCandidateSummary> =
            serde_json::from_slice(&body).expect("compression candidates JSON");
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
    }

    #[test]
    fn compression_detail_keeps_unattempted_recovery_and_determinism_unavailable() {
        let fixture = fixture_database(false).expect("fixture");
        let connection = rusqlite::Connection::open(fixture.path()).expect("fixture writer");
        let _updated = connection
            .execute(
                "UPDATE compression_candidate_metrics
                    SET output_bytes = NULL, bytes_delta = NULL,
                        output_estimated_tokens = NULL, estimated_token_delta = NULL
                  WHERE candidate_id IN (
                        SELECT candidate_id FROM compression_candidates
                         WHERE compressor_id = 'json.repeated_subtree'
                  )",
                [],
            )
            .expect("make every repeated-subtree candidate unattempted");

        let detail = db::compression_experiment(&connection, "shadow-pilot-001")
            .expect("compression detail query")
            .expect("experiment");
        let repeated_subtree = detail
            .compressors
            .iter()
            .find(|summary| summary.compressor == "json.repeated_subtree")
            .expect("json.repeated_subtree summary");
        assert_eq!(repeated_subtree.recovery_basis_points, None);
        assert_eq!(repeated_subtree.deterministic_basis_points, None);
    }

    #[tokio::test]
    async fn empty_database_returns_zero_counts_and_unavailable_metrics() {
        let fixture = fixture_database(false).expect("fixture");
        let connection = rusqlite::Connection::open(fixture.path()).expect("fixture writer");
        connection
            .execute_batch(
                "DELETE FROM compression_recoveries;
                 DELETE FROM compression_candidate_metrics;
                 DELETE FROM compression_candidates;
                 DELETE FROM compression_experiments;
                 DELETE FROM context_deltas;
                 DELETE FROM context_analysis_metrics;
                 DELETE FROM context_block_occurrences;
                 DELETE FROM context_snapshots;
                 DELETE FROM provider_usage;
                 DELETE FROM provider_attempts;
                 DELETE FROM provider_requests;
                 DELETE FROM operations;
                 DELETE FROM sessions;",
            )
            .expect("clear synthetic rows");
        drop(connection);
        let reports = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reports");
        let response = router(fixture.path().to_path_buf(), reports)
            .oneshot(
                Request::builder()
                    .uri("/api/v1/overview")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let overview: Overview = serde_json::from_slice(&bytes).expect("overview JSON");
        assert_eq!(overview.sessions, 0);
        assert_eq!(overview.provider_requests, 0);
        assert_eq!(overview.usage.input_tokens.value, None);
        assert_eq!(overview.quality.analysis_coverage, None);
    }

    #[tokio::test]
    async fn baseline_artifacts_load_without_recalculating_convergence() {
        let (status, body) = response("/api/v1/baselines/baseline-004", false).await;
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let detail: tracepress_dashboard_types::BaselineDetail =
            serde_json::from_slice(&body).expect("baseline JSON");
        assert_eq!(detail.summary.status, "converged");
        assert_eq!(detail.summary.sessions, Some(40));
        assert_eq!(detail.convergence.len(), 4);
        assert_eq!(
            detail.convergence.last().and_then(|point| point.stable),
            Some(true)
        );
    }
}
