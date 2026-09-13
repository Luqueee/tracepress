//! Dioxus 0.7 WASM frontend for the local Tracepress Observatory.
#![allow(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    clippy::enum_variant_names,
    clippy::exhaustive_enums,
    clippy::format_push_string,
    clippy::future_not_send,
    clippy::min_ident_chars,
    clippy::missing_const_for_fn,
    clippy::multiple_crate_versions,
    clippy::needless_pass_by_value,
    clippy::option_if_let_else,
    clippy::ref_option,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::volatile_composites,
    reason = "Dioxus components are declarative UI boundaries and chart coordinates use bounded display values"
)]

use dioxus::prelude::*;
use gloo_net::http::Request;
use serde::de::DeserializeOwned;
use tracepress_dashboard_types::{
    BaselineDetail, BaselineSummary, CompressionCandidateSummary, CompressionExperimentDetail,
    CompressionExperimentSummary, CompressionHistogramBucket, ContextCategoryStats,
    ContextExplorer, ContextGrowthPoint, MeasurementQuality, Metric as WireMetric, MetricSource,
    OpportunitySummary, Overview, Page, SessionContext, SessionDetail, SessionSummary,
    UnknownSummary, WorkloadSummary,
};

const MAIN_CSS: Asset = asset!("/assets/main.css");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
const UNAVAILABLE: &str = "Unavailable";
const REASONING_UNAVAILABLE: &str = "Reasoning unavailable";
const TRANSPORT_UNAVAILABLE: &str = "Transport unavailable";
const RUNNING: &str = "RUNNING";

fn main() {
    #[cfg(target_arch = "wasm32")]
    install_csp_safe_logger();
    dioxus::launch(app);
}

#[cfg(target_arch = "wasm32")]
fn install_csp_safe_logger() {
    use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};

    let mut config = tracing_wasm::WASMLayerConfigBuilder::new();
    let _configured =
        config.set_console_config(tracing_wasm::ConsoleConfig::ReportWithoutConsoleColor);
    let layer = tracing_wasm::WASMLayer::new(config.build());
    let _ = tracing_subscriber::registry().with(layer).try_init();
}

fn app() -> Element {
    rsx! {
        document::Stylesheet { href: MAIN_CSS }
        document::Stylesheet { href: TAILWIND_CSS }
        document::Title { "Tracepress Observatory" }
        Router::<Route> {}
    }
}

#[derive(Clone, Debug, PartialEq, Routable)]
#[rustfmt::skip]
enum Route {
    #[layout(AppShell)]
        #[route("/")] OverviewPage {},
        #[route("/sessions")] SessionsPage {},
        #[route("/sessions/:id")] SessionDetailPage { id: String },
        #[route("/context")] ContextPage {},
        #[route("/workloads")] WorkloadsPage {},
        #[route("/baselines")] BaselinesPage {},
        #[route("/baselines/:id")] BaselineDetailPage { id: String },
        #[route("/opportunities")] OpportunitiesPage {},
        #[route("/compression")] CompressionPage {},
        #[route("/compression/:id")] CompressionDetailPage { id: String },
    #[end_layout]
    #[route("/:..route")] NotFoundPage { route: Vec<String> },
}

#[component]
fn AppShell() -> Element {
    rsx! { div { class: "app-shell min-h-screen bg-background text-text-primary", Sidebar {} div { class: "workspace", TopBar {} main { Outlet::<Route> {} } } } }
}

#[component]
fn Sidebar() -> Element {
    rsx! { aside { class: "sidebar bg-surface-1 border-border", aria_label: "Primary navigation",
        div { class: "brand", span { class: "brand-mark", aria_hidden: "true" } "Tracepress" }
        nav {
            Link { class: "nav-link", to: Route::OverviewPage {}, "Overview" }
            div { class: "nav-label", "Explore" }
            Link { class: "nav-link", to: Route::SessionsPage {}, "Sessions" }
            Link { class: "nav-link", to: Route::ContextPage {}, "Context" }
            Link { class: "nav-link", to: Route::WorkloadsPage {}, "Workloads" }
            div { class: "nav-label", "Research" }
            Link { class: "nav-link", to: Route::BaselinesPage {}, "Baselines" }
            Link { class: "nav-link", to: Route::OpportunitiesPage {}, "Opportunities" }
            Link { class: "nav-link", to: Route::CompressionPage {}, "Compression" span { class: "badge success", "Shadow" } }
        }
    } }
}

#[component]
fn TopBar() -> Element {
    rsx! { header { class: "topbar",
        div { class: "topbar-meta", Badge { text: "LOCAL", tone: "success" } span { "Read-only observatory" } }
        div { class: "topbar-meta", span { class: "mono", "127.0.0.1:4319" } IconButton { label: "Refresh current view", icon: "↻" } }
    } }
}

#[component]
fn Button(label: String, #[props(default = false)] disabled: bool) -> Element {
    rsx! { button { class: "button", disabled, "{label}" } }
}

#[component]
fn IconButton(label: &'static str, icon: &'static str) -> Element {
    rsx! { button { class: "button", aria_label: label, title: label, "{icon}" } }
}

#[component]
fn Badge(text: String, #[props(default = "neutral".to_owned())] tone: String) -> Element {
    rsx! { span { class: "badge {tone}", "{text}" } }
}

#[component]
fn Card(title: String, children: Element) -> Element {
    rsx! { section { class: "card", div { class: "card-header", h2 { class: "section-title", "{title}" } } div { class: "card-body", {children} } } }
}

#[component]
fn Metric(
    label: String,
    value: String,
    source: String,
    #[props(default)] exact: Option<String>,
) -> Element {
    let title = exact.unwrap_or_else(|| value.clone());
    rsx! { div { class: "card metric", title,
        div { class: "metric-label", "{label}" }
        div { class: "metric-value", "{value}" }
        div { class: "metric-source", "{source}" }
    } }
}

#[component]
fn MetricGroup(children: Element) -> Element {
    rsx! { div { class: "grid metrics", {children} } }
}

#[component]
fn DataTable(children: Element) -> Element {
    rsx! { div { class: "table-wrap", table { {children} } } }
}

#[component]
fn Pagination(next: Option<String>, cursor: Signal<Option<String>>) -> Element {
    let current_offset = cursor
        .read()
        .as_deref()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let has_previous = current_offset > 0;
    let previous = current_offset.saturating_sub(50);
    let page_number = current_offset / 50 + 1;
    let next_value = next.clone();
    rsx! { div { class: "pagination",
        button { class: "button", disabled: !has_previous, onclick: move |_| { let mut cursor = cursor; cursor.set(if previous == 0 { None } else { Some(previous.to_string()) }); }, "Previous" }
        span { "Page {page_number}" }
        button { class: "button", disabled: next.is_none(), onclick: move |_| { let mut cursor = cursor; cursor.set(next_value.clone()); }, "Next" }
    } }
}

#[component]
fn Tabs(labels: Vec<String>, active: String) -> Element {
    rsx! { div { class: "tabs", for label in labels { span { class: if label == active { "tab active" } else { "tab" }, "{label}" } } } }
}

#[component]
fn Select(label: String, children: Element) -> Element {
    rsx! { label { class: "secondary", span { class: "muted", "{label} " } select { class: "select", {children} } } }
}

#[component]
fn FilterBar(children: Element) -> Element {
    rsx! { div { class: "filter-bar", {children} } }
}

#[component]
fn Tooltip(text: String) -> Element {
    rsx! { span { class: "badge", tabindex: "0", title: text.clone(), aria_label: text, "?" } }
}

#[component]
fn Skeleton() -> Element {
    rsx! { div { class: "state", div { class: "stack", aria_label: "Loading", div { class: "skeleton" } div { class: "skeleton" } div { class: "skeleton" } } } }
}

#[component]
fn EmptyState(title: String, message: String) -> Element {
    rsx! { div { class: "state", div { h2 { "{title}" } p { "{message}" } } } }
}

#[component]
fn ErrorState(message: String) -> Element {
    rsx! { div { class: "state", div { h2 { "Could not load this view" } p { "{message}" } Button { label: "Try again" } } } }
}

#[component]
fn ChartContainer(title: String, meta: String, children: Element) -> Element {
    rsx! { Card { title, div { class: "secondary chart-meta", "{meta}" } {children} } }
}

#[component]
fn Legend() -> Element {
    rsx! { div { class: "legend",
        span { i { class: "legend-key" } "Provider input" }
        span { i { class: "legend-key cached" } "Cached input" }
        span { i { class: "legend-key uncached" } "Uncached input" }
        span { i { class: "legend-key estimated" } "Estimated explicit context" }
    } }
}

#[component]
fn PageHeader(title: String, subtitle: String) -> Element {
    rsx! { div { class: "page page-heading", div { class: "page-header", div { h1 { class: "page-title", "{title}" } p { class: "page-subtitle", "{subtitle}" } } } } }
}

#[component]
fn OverviewPage() -> Element {
    let resource = use_resource(|| fetch::<Overview>("/api/v1/overview"));
    rsx! { PageHeader { title: "Overview", subtitle: "Provider usage, context structure, repetition, and measurement integrity at a glance." }
        div { class: "page", {render_overview(&resource.read())} }
    }
}

fn render_overview(state: &Option<Result<Overview, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(data)) => rsx! { OverviewContent { data: data.clone() } },
    }
}

#[component]
fn OverviewContent(data: Overview) -> Element {
    rsx! {
        QualityBanner { quality: data.quality.clone() }
        MetricGroup {
            Metric { label: "Sessions", value: compact_u64(data.sessions), exact: data.sessions.to_string(), source: "Operational SQLite" }
            Metric { label: "Provider requests", value: compact_u64(data.provider_requests), exact: data.provider_requests.to_string(), source: "Operational SQLite" }
            MetricFromU64 { label: "Input tokens", metric: data.usage.input_tokens.clone() }
            MetricFromU64 { label: "Cached input", metric: data.usage.cached_input_tokens.clone() }
            MetricFromU64 { label: "Uncached input", metric: data.usage.uncached_input_tokens.clone() }
            MetricFromRatio { label: "Cache ratio", metric: data.usage.cache_ratio.clone() }
            MetricFromU64 { label: "Output", metric: data.usage.output_tokens.clone() }
            MetricFromU64 { label: "Reasoning", metric: data.usage.reasoning_tokens.clone() }
            MetricFromRatio { label: "Analysis coverage", metric: WireMetric::new(data.quality.analysis_coverage, MetricSource::LocallyEstimated) }
            MetricFromRatio { label: "Correlation coverage", metric: WireMetric::new(data.quality.correlation_coverage, MetricSource::LocallyEstimated) }
            Metric { label: "Drops", value: compact_u64(data.quality.dropped_requests), exact: data.quality.dropped_requests.to_string(), source: "Observed" }
            Metric { label: "Malformed", value: compact_u64(data.quality.malformed_requests), exact: data.quality.malformed_requests.to_string(), source: "Observed" }
        }
        div { class: "grid two spacer-top",
            BarChart { title: "Context composition", rows: data.composition.categories, coverage: data.composition.estimator_coverage }
            RepetitionCard { repetition: data.repetition }
        }
    }
}

#[component]
fn QualityBanner(quality: MeasurementQuality) -> Element {
    let healthy = quality.status == "healthy";
    rsx! { div { class: if healthy { "quality-banner" } else { "quality-banner degraded" }, role: "status",
        Badge { text: if healthy { "Healthy".to_owned() } else { "Degraded".to_owned() }, tone: if healthy { "success".to_owned() } else { "warning".to_owned() } }
        div { strong { "Measurement integrity: " if healthy { "Healthy" } else { "Coverage degraded" } }
            if !quality.reasons.is_empty() { div { class: "secondary", "{join_reasons(&quality.reasons)}" } }
        }
    } }
}

#[component]
fn BarChart(title: String, rows: Vec<ContextCategoryStats>, coverage: Option<f64>) -> Element {
    rsx! { ChartContainer { title, meta: format!("Estimator coverage: {}", format_ratio(coverage)),
        div { class: "bar-list", for row in rows.iter().take(8) {
            div { class: "bar-row", span { "{humanize(&row.name)}" }
                BarGauge { value: row.estimated_token_share, label: format!("{} {}", humanize(&row.name), format_ratio(row.estimated_token_share)), tone: "" }
                span { class: "number", "{format_ratio(row.estimated_token_share)}" }
            }
        } }
    } }
}

#[component]
fn RepetitionCard(repetition: tracepress_dashboard_types::RepetitionSummary) -> Element {
    rsx! { ChartContainer { title: "Repetition and cache", meta: "Correlation does not imply provider cache equivalence.",
        div { class: "stack", RatioBar { label: "Exact repeated", metric: repetition.exact_token_share }
            RatioBar { label: "Semantic repeated", metric: repetition.semantic_token_share }
            RatioBar { label: "Provider cache", metric: repetition.provider_cache_ratio }
        }
    } }
}

#[component]
fn RatioBar(label: String, metric: WireMetric<f64>) -> Element {
    rsx! { div { class: "bar-row", span { "{label}" }
        BarGauge { value: metric.value, label: if metric.value.is_some() { format!("{label} {value}", value = format_ratio(metric.value)) } else { format!("{label} unavailable") }, tone: "" }
        span { class: "number", "{format_ratio(metric.value)}" }
    } }
}

#[component]
fn BarGauge(value: Option<f64>, label: String, tone: String) -> Element {
    let width = value
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(0.0, 1.0) * 100.0);
    rsx! { div { class: if width.is_some() { "bar-track" } else { "bar-track unavailable" }, aria_label: label,
        if let Some(width) = width {
            svg { class: "bar-gauge", view_box: "0 0 100 5", preserve_aspect_ratio: "none", role: "presentation",
                rect { class: "bar-fill {tone}", x: "0", y: "0", width: "{width:.2}", height: "5" }
            }
        }
    } }
}

#[component]
fn SessionsPage() -> Element {
    let cursor = use_signal(|| None::<String>);
    let mut model = use_signal(String::new);
    let mut status = use_signal(String::new);
    let mut sort = use_signal(|| "created_at".to_owned());
    let resource = use_resource(move || {
        let cursor_value = cursor.read().clone();
        let model_value = model.read().clone();
        let status_value = status.read().clone();
        let sort_value = sort.read().clone();
        async move {
            let mut url = format!("/api/v1/sessions?limit=50&sort={sort_value}");
            if let Some(value) = cursor_value {
                url.push_str(&format!("&cursor={value}"));
            }
            if !model_value.is_empty() {
                url.push_str(&format!("&model={model_value}"));
            }
            if !status_value.is_empty() {
                url.push_str(&format!("&status={status_value}"));
            }
            fetch::<Page<SessionSummary>>(&url).await
        }
    });
    rsx! { PageHeader { title: "Sessions", subtitle: "Operational sessions with provider usage and context-analysis metadata." }
        div { class: "page",
            FilterBar {
                label { "Status " select { class: "select", value: "{status}", onchange: move |event| status.set(event.value()), option { value: "", "All" } option { value: "complete", "Complete" } option { value: "running", "Running" } } }
                label { "Sort " select { class: "select", value: "{sort}", onchange: move |event| sort.set(event.value()), option { value: "created_at", "Created" } option { value: "request_count", "Requests" } option { value: "input_tokens", "Input" } option { value: "cache_ratio", "Cache ratio" } option { value: "estimated_context_tokens", "Context" } option { value: "repetition", "Repetition" } } }
                label { "Model " input { class: "input", value: "{model}", placeholder: "exact model", oninput: move |event| model.set(event.value()) } }
            }
            {render_sessions(&resource.read(), cursor)}
        }
    }
}

fn render_sessions(
    state: &Option<Result<Page<SessionSummary>, String>>,
    cursor: Signal<Option<String>>,
) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(page)) if page.items.is_empty() => {
            rsx! { EmptyState { title: "No sessions", message: "No operational sessions match the current filters." } }
        }
        Some(Ok(page)) => {
            rsx! { SessionsTable { sessions: page.items.clone() } Pagination { next: page.next_cursor.clone(), cursor } }
        }
    }
}

#[component]
fn SessionsTable(sessions: Vec<SessionSummary>) -> Element {
    rsx! { DataTable {
        thead { tr { th { "Session" } th { "Workload" } th { class: "numeric", "Requests" } th { class: "numeric", "Input" } th { class: "numeric", "Cached" } th { class: "numeric", "Cache %" } th { class: "numeric", "Output" } th { class: "numeric", "Repetition" } th { class: "numeric", "Semantic" } th { class: "numeric", "Duration" } th { "Status" } } }
        tbody { for session in sessions { tr {
            td { Link { class: "mono", title: session.id.clone(), to: Route::SessionDetailPage { id: session.id.clone() }, "{short_id(&session.id)}" } }
            td { "{optional_text(&session.workload)}" } td { class: "numeric", "{session.request_count}" }
            td { class: "numeric", "{format_metric_u64(&session.usage.input_tokens)}" } td { class: "numeric", "{format_metric_u64(&session.usage.cached_input_tokens)}" }
            td { class: "numeric", "{format_metric_ratio(&session.usage.cache_ratio)}" } td { class: "numeric", "{format_metric_u64(&session.usage.output_tokens)}" }
            td { class: "numeric", "{format_metric_ratio(&session.repetition)}" } td { class: "numeric", "{format_metric_ratio(&session.semantic_coverage)}" }
            td { class: "numeric", "{format_duration(session.duration_us)}" } td { Badge { text: session.status.clone(), tone: status_tone(&session.status) } }
        } } }
    } }
}

#[component]
fn SessionDetailPage(id: String) -> Element {
    let request_id = id.clone();
    let resource = use_resource(move || {
        let url = format!("/api/v1/sessions/{request_id}");
        async move { fetch::<SessionDetail>(&url).await }
    });
    let context_id = id.clone();
    let context = use_resource(move || {
        let url = format!("/api/v1/sessions/{context_id}/context?limit=50");
        async move { fetch::<SessionContext>(&url).await }
    });
    rsx! { PageHeader { title: "Session", subtitle: id } div { class: "page", {render_session_detail(&resource.read())} div { class: "spacer-top", {render_session_context(&context.read())} } } }
}

fn render_session_detail(state: &Option<Result<SessionDetail, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(detail)) => rsx! { SessionDetailContent { detail: detail.clone() } },
    }
}

fn render_session_context(state: &Option<Result<SessionContext, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(context)) if context.blocks.items.is_empty() => rsx! {
            EmptyState { title: "No context blocks", message: "No analyzed context snapshot is available for this session." }
        },
        Some(Ok(context)) => rsx! { Card { title: "Latest request context blocks",
            Tabs { labels: vec!["Blocks".to_owned(), "Composition".to_owned()], active: "Blocks" }
            DataTable {
                thead { tr { th { "Ordinal" } th { "Kind" } th { "Role" } th { "Origin" } th { "Detected" } th { class: "numeric", "Raw bytes" } th { class: "numeric", "Estimated tokens" } th { "Exact rep." } th { "Semantic rep." } th { class: "numeric", "Persistence" } th { "Fingerprint" } } }
                tbody { for block in &context.blocks.items { tr {
                    td { class: "numeric", "{block.ordinal}" } td { "{humanize(&block.kind)}" } td { "{humanize(&block.role)}" } td { "{humanize(&block.origin)}" }
                    td { "{block.detected_kind.as_deref().unwrap_or(UNAVAILABLE)}" } td { class: "numeric", "{compact_u64(block.raw_bytes)}" } td { class: "numeric", "{format_optional_u64(block.estimated_tokens)}" }
                    td { "{optional_bool(block.exact_repeated)}" } td { "{optional_bool(block.semantic_repeated)}" } td { class: "numeric", "{format_optional_u64(block.persistence)}" }
                    td { class: "mono", "{optional_text(&block.fingerprint_short)}" }
                } } }
            }
        } },
    }
}

#[component]
#[allow(
    clippy::redundant_clone,
    reason = "Dioxus event handlers are FnMut and retain the selected request id across invocations"
)]
fn SessionDetailContent(detail: SessionDetail) -> Element {
    let first_request = detail.requests.first().map(|request| request.id.clone());
    let mut selected_request = use_signal(|| first_request);
    let selected = detail
        .requests
        .iter()
        .find(|request| Some(&request.id) == selected_request.read().as_ref())
        .cloned();
    rsx! {
        FilterBar { Badge { text: detail.session.status.clone(), tone: status_tone(&detail.session.status) }
            span { class: "mono", "{optional_label(&detail.session.model, UNAVAILABLE)}" }
            span { "{optional_label(&detail.reasoning_effort, REASONING_UNAVAILABLE)}" }
            span { "{optional_label(&detail.session.transport, TRANSPORT_UNAVAILABLE)}" }
            span { "{detail.session.started_at} → {optional_label(&detail.session.ended_at, RUNNING)}" }
        }
        MetricGroup {
            Metric { label: "Requests", value: detail.session.request_count.to_string(), source: "Operational SQLite" }
            MetricFromU64 { label: "Provider input", metric: detail.session.usage.input_tokens.clone() }
            MetricFromU64 { label: "Cached", metric: detail.session.usage.cached_input_tokens.clone() }
            MetricFromU64 { label: "Uncached", metric: detail.session.usage.uncached_input_tokens.clone() }
            MetricFromU64 { label: "Output", metric: detail.session.usage.output_tokens.clone() }
            MetricFromU64 { label: "Reasoning", metric: detail.session.usage.reasoning_tokens.clone() }
            MetricFromU64 { label: "Visible context", metric: detail.session.estimated_context_tokens.clone() }
            MetricFromRatio { label: "Repetition", metric: detail.session.repetition.clone() }
        }
        div { class: "spacer-top", ContextGrowthChart { points: detail.context_growth } }
        div { class: "spacer-top", Card { title: "Request timeline", div { class: "timeline", for request in detail.requests {
            button { class: if selected_request.read().as_ref() == Some(&request.id) { "timeline-row button-row selected" } else { "timeline-row button-row" }, aria_label: format!("Inspect request {}", request.ordinal), onclick: { let id = request.id.clone(); move |_| selected_request.set(Some(id.clone())) }, span { class: "mono", "#{request.ordinal}" } span { Badge { text: request.kind.clone(), tone: if request.kind.starts_with("compaction") { "warning".to_owned() } else { "neutral".to_owned() } } }
                span { class: "number", "In {format_metric_u64(&request.usage.input_tokens)}" } span { class: "number", "Cached {format_metric_u64(&request.usage.cached_input_tokens)}" }
                span { class: "number", "Out {format_metric_u64(&request.usage.output_tokens)}" } span { class: "number", "Context {format_metric_u64(&request.estimated_context_tokens)}" }
                span { Badge { text: request.status.clone(), tone: status_tone(&request.status) } }
            }
        } } } }
        if let Some(request) = selected { div { class: "spacer-top", RequestDetailPanel { request } } }
    }
}

#[component]
fn RequestDetailPanel(request: tracepress_dashboard_types::ProviderRequestSummary) -> Element {
    rsx! { Card { title: format!("Request #{}", request.ordinal),
        div { class: "grid three",
            div { h3 { class: "section-kicker", "Provider observation" } p { class: "mono", title: request.id, "{short_id(&request.id)}" } p { "Model: {optional_text(&request.model)}" } p { "Status: {request.status}" } }
            div { h3 { class: "section-kicker", "Usage" } p { "Input: {format_metric_u64(&request.usage.input_tokens)}" } p { "Cached: {format_metric_u64(&request.usage.cached_input_tokens)}" } p { "Output: {format_metric_u64(&request.usage.output_tokens)}" } }
            div { h3 { class: "section-kicker", "Context observation" } p { "Visible estimate: {format_metric_u64(&request.estimated_context_tokens)}" } p { "Semantic coverage: {format_metric_ratio(&request.semantic_coverage)}" } p { "Duration: {format_duration(request.duration_us)}" } }
            div { h3 { class: "section-kicker", "Visibility" } p { class: "muted", "Unavailable in request summary" } }
            div { h3 { class: "section-kicker", "Composition and repetition" } p { class: "muted", "Use the metadata-only context table below; absent observations are not inferred." } }
            div { h3 { class: "section-kicker", "Scheduler" } p { class: "muted", "Unavailable" } }
        }
    } }
}

#[component]
fn ContextGrowthChart(points: Vec<ContextGrowthPoint>) -> Element {
    let max = points
        .iter()
        .flat_map(|point| [point.provider_input_tokens, point.estimated_context_tokens])
        .flatten()
        .max()
        .unwrap_or(1) as f64;
    let provider = polyline_segments(&points, max, |point| point.provider_input_tokens);
    let cached = polyline_segments(&points, max, |point| point.cached_input_tokens);
    let uncached = polyline_segments(&points, max, |point| point.uncached_input_tokens);
    let estimated = polyline_segments(&points, max, |point| point.estimated_context_tokens);
    rsx! { ChartContainer { title: "Context growth", meta: "Provider input and locally estimated explicit context. Missing samples create gaps.",
        svg { class: "chart", view_box: "0 0 800 230", role: "img",
            title { "Context growth by request ordinal" }
            for y in [30, 80, 130, 180] { line { class: "chart-grid", x1: "40", y1: "{y}", x2: "780", y2: "{y}" } }
            for segment in provider { polyline { class: "chart-line-provider", points: segment } }
            for segment in cached { polyline { class: "chart-line-cached", points: segment } }
            for segment in uncached { polyline { class: "chart-line-uncached", points: segment } }
            for segment in estimated { polyline { class: "chart-line-estimated", points: segment } }
        } Legend {}
    } }
}

#[component]
fn ContextPage() -> Element {
    let mut group = use_signal(|| "detected_kind".to_owned());
    let resource = use_resource(move || {
        let selected = group.read().clone();
        async move {
            fetch::<ContextExplorer>(&format!("/api/v1/context/composition?group_by={selected}"))
                .await
        }
    });
    let unknown = use_resource(|| fetch::<UnknownSummary>("/api/v1/context/unknown"));
    rsx! { PageHeader { title: "Context Explorer", subtitle: "Metadata-only composition, repetition, persistence, and Origin × DetectedContentKind." }
        div { class: "page", FilterBar { label { "Group by " select { class: "select", value: "{group}", onchange: move |event| group.set(event.value()), option { value: "detected_kind", "Detected content" } option { value: "kind", "Block kind" } option { value: "origin", "Origin" } option { value: "role", "Role" } } } }
            {render_context(&resource.read())}
            div { class: "spacer-top", {render_unknown(&unknown.read())} }
        }
    }
}

fn render_context(state: &Option<Result<ContextExplorer, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(data)) => {
            rsx! { BarChart { title: format!("Composition by {}", humanize(&data.composition.group_by)), rows: data.composition.categories.clone(), coverage: data.composition.estimator_coverage } div { class: "spacer-top", ContextMatrix { cells: data.matrix.clone() } } }
        }
    }
}

fn render_unknown(state: &Option<Result<UnknownSummary, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(data)) => rsx! { UnknownPanel { data: data.clone() } },
    }
}

#[component]
fn ContextMatrix(cells: Vec<tracepress_dashboard_types::ContextMatrixCell>) -> Element {
    let mut origins = cells
        .iter()
        .map(|cell| cell.origin.clone())
        .collect::<Vec<_>>();
    origins.sort();
    origins.dedup();
    let mut kinds = cells
        .iter()
        .map(|cell| cell.detected_kind.clone())
        .collect::<Vec<_>>();
    kinds.sort();
    kinds.dedup();
    rsx! { Card { title: "Origin × DetectedContentKind", DataTable {
        thead { tr { th { "Origin" } for kind in &kinds { th { class: "numeric", "{humanize(kind)}" } } } }
        tbody { for origin in &origins { tr { td { "{humanize(origin)}" } for kind in &kinds { td { class: "numeric matrix",
            if let Some(cell) = cells.iter().find(|cell| &cell.origin == origin && &cell.detected_kind == kind) {
                span { class: strength_class(cell.estimated_token_share), title: format!("{} estimated tokens", format_optional_u64(cell.estimated_tokens)), "{format_ratio(cell.estimated_token_share)}" }
            } else { "—" }
        } } } } }
    } } }
}

#[component]
fn UnknownPanel(data: UnknownSummary) -> Element {
    rsx! { Card { title: "Unknown Explorer", p { class: "secondary", "Unknown is descriptive evidence, not an automatic compression recommendation." }
        MetricGroup {
            Metric { label: "Blocks", value: compact_u64(data.blocks), exact: data.blocks.to_string(), source: "Observed" }
            Metric { label: "Estimated tokens", value: format_optional_u64(data.estimated_tokens), source: "Locally estimated" }
            Metric { label: "Estimator coverage", value: format_ratio(data.estimator_coverage), source: "Locally estimated" }
            Metric { label: "Raw bytes", value: compact_u64(data.raw_bytes), exact: data.raw_bytes.to_string(), source: "Observed" }
            Metric { label: "Unique exact", value: compact_u64(data.unique_exact_fingerprints), source: "Fingerprint metadata" }
            Metric { label: "Unique semantic", value: compact_u64(data.unique_semantic_fingerprints), source: "Fingerprint metadata" }
            Metric { label: "P50 size", value: format_optional_u64(data.size_percentiles.p50), source: "Raw bytes" }
            Metric { label: "P90 size", value: format_optional_u64(data.size_percentiles.p90), source: "Raw bytes" }
            Metric { label: "P99 size", value: format_optional_u64(data.size_percentiles.p99), source: "Raw bytes" }
            Metric { label: "Persistence", value: data.persistence.map_or_else(|| "—".to_owned(), |value| format!("{value:.2}×")), source: "Observed fingerprints" }
        }
    } }
}

#[component]
fn WorkloadsPage() -> Element {
    let resource = use_resource(|| fetch::<Vec<WorkloadSummary>>("/api/v1/workloads"));
    rsx! { PageHeader { title: "Workloads", subtitle: "Certified baseline workload comparison. Operational sessions remain independent from report artifacts." } div { class: "page", {render_workloads(&resource.read())} } }
}

fn render_workloads(state: &Option<Result<Vec<WorkloadSummary>, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(rows)) if rows.is_empty() => {
            rsx! { EmptyState { title: "No workload metadata", message: "No certified workload report is available." } }
        }
        Some(Ok(rows)) => {
            rsx! { WorkloadBars { rows: rows.clone() } WorkloadsTable { rows: rows.clone() } }
        }
    }
}

#[component]
fn WorkloadBars(rows: Vec<WorkloadSummary>) -> Element {
    rsx! { ChartContainer { title: "Workload composition", meta: "Grouped comparison; unavailable values use dashed empty tracks, not zero.",
        div { class: "workload-bars",
            div { class: "workload-bar-row section-kicker", span { "Workload" } span { "JSON" } span { "Plain" } span { "Unknown" } span { "Cache" } }
            for row in rows {
                div { class: "workload-bar-row", span { "{humanize(&row.workload)}" }
                    MiniBar { label: "JSON", value: row.json_share, tone: "" }
                    MiniBar { label: "Plain", value: row.plain_share, tone: "plain" }
                    MiniBar { label: "Unknown", value: row.unknown_share, tone: "unknown" }
                    MiniBar { label: "Cache", value: row.cache_ratio, tone: "cache" }
                }
            }
        }
    } }
}

#[component]
fn MiniBar(label: &'static str, value: Option<f64>, tone: &'static str) -> Element {
    rsx! { div { class: "mini-bar", title: format!("{label}: {}", format_ratio(value)),
        BarGauge { value, label: format!("{label}: {}", format_ratio(value)), tone }
        span { class: "number", "{format_ratio(value)}" }
    } }
}

#[component]
fn WorkloadsTable(rows: Vec<WorkloadSummary>) -> Element {
    rsx! { DataTable { thead { tr { th { "Workload" } th { class: "numeric", "Sessions" } th { class: "numeric", "Requests" } th { class: "numeric", "Token share" } th { class: "numeric", "JSON" } th { class: "numeric", "Plain" } th { class: "numeric", "Unknown" } th { class: "numeric", "Cache" } th { class: "numeric", "Exact rep." } th { class: "numeric", "Semantic rep." } } }
        tbody { for row in rows { tr { td { "{humanize(&row.workload)}" } td { class: "numeric", "{row.sessions}" } td { class: "numeric", "{optional_count(row.requests)}" }
            td { class: "numeric", "{format_ratio(row.token_share)}" } td { class: "numeric", "{format_ratio(row.json_share)}" } td { class: "numeric", "{format_ratio(row.plain_share)}" }
            td { class: "numeric", "{format_ratio(row.unknown_share)}" } td { class: "numeric", "{format_ratio(row.cache_ratio)}" } td { class: "numeric", "{format_ratio(row.exact_repetition)}" } td { class: "numeric", "{format_ratio(row.semantic_repetition)}" }
        } } }
    } }
}

#[component]
fn BaselinesPage() -> Element {
    let resource = use_resource(|| fetch::<Vec<BaselineSummary>>("/api/v1/baselines"));
    rsx! { PageHeader { title: "Baselines", subtitle: "Reproducible artifacts remain the source of truth; Observatory explores safe projections." } div { class: "page", {render_baselines(&resource.read())} } }
}

fn render_baselines(state: &Option<Result<Vec<BaselineSummary>, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(rows)) => rsx! { DataTable {
            thead { tr { th { "Baseline" } th { "Certification" } th { "Cohort" } th { class: "numeric", "Sessions" } th { class: "numeric", "Requests" } th { "Integrity" } } }
            tbody { for row in rows { tr { td { Link { class: "mono", to: Route::BaselineDetailPage { id: row.id.clone() }, "{row.id}" } } td { Badge { text: row.status.clone(), tone: baseline_tone(&row.status) } }
                td { "{optional_text(&row.cohort)}" } td { class: "numeric", "{format_optional_u64(row.sessions)}" } td { class: "numeric", "{format_optional_u64(row.requests)}" } td { "{optional_label(&row.measurement_integrity, UNAVAILABLE)}" }
            } } }
        } },
    }
}

#[component]
fn BaselineDetailPage(id: String) -> Element {
    let baseline_id = id.clone();
    let resource = use_resource(move || {
        let url = format!("/api/v1/baselines/{baseline_id}");
        async move { fetch::<BaselineDetail>(&url).await }
    });
    rsx! { PageHeader { title: "Baseline detail", subtitle: id } div { class: "page", {render_baseline_detail(&resource.read())} } }
}

fn render_baseline_detail(state: &Option<Result<BaselineDetail, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(detail)) => rsx! { BaselineContent { detail: detail.clone() } },
    }
}

#[component]
fn BaselineContent(detail: BaselineDetail) -> Element {
    rsx! {
        FilterBar { Badge { text: detail.summary.status.clone(), tone: baseline_tone(&detail.summary.status) }
            span { "Codex {optional_text(&detail.manifest.codex_version)}" } span { "Model {optional_text(&detail.manifest.model)}" }
            span { "Reasoning {optional_text(&detail.manifest.reasoning_effort)}" }
            span { class: "mono", title: detail.manifest.instrument_sha.clone().unwrap_or_default(), "Instrument {optional_short_id(&detail.manifest.instrument_sha)}" }
        }
        if let Some(quality) = detail.quality { QualityBanner { quality } }
        if let Some(usage) = detail.usage { MetricGroup { MetricFromU64 { label: "Input", metric: usage.input_tokens } MetricFromU64 { label: "Cached", metric: usage.cached_input_tokens } MetricFromRatio { label: "Cache ratio", metric: usage.cache_ratio } MetricFromU64 { label: "Output", metric: usage.output_tokens } MetricFromU64 { label: "Reasoning", metric: usage.reasoning_tokens } } }
        div { class: "spacer-top", ConvergenceTable { points: detail.convergence } }
        div { class: "spacer-top", WorkloadsTable { rows: detail.workloads } }
    }
}

#[component]
fn ConvergenceTable(points: Vec<tracepress_dashboard_types::ConvergencePoint>) -> Element {
    let json = convergence_segments(&points, |point| point.json_share);
    let plain = convergence_segments(&points, |point| point.plain_share);
    let unknown = convergence_segments(&points, |point| point.unknown_share);
    let cache = convergence_segments(&points, |point| point.cache_ratio);
    let repetition = convergence_segments(&points, |point| point.repetition);
    rsx! { ChartContainer { title: "Official convergence", meta: "Stability comes from the official convergence artifact; the UI does not recalculate gates.",
        svg { class: "chart", view_box: "0 0 800 230", role: "img", title { "Baseline convergence across cohorts" }
            for y in [30, 80, 130, 180] { line { class: "chart-grid", x1: "40", y1: "{y}", x2: "780", y2: "{y}" } }
            for segment in json { polyline { class: "chart-line-json", points: segment } }
            for segment in plain { polyline { class: "chart-line-plain", points: segment } }
            for segment in unknown { polyline { class: "chart-line-unknown", points: segment } }
            for segment in cache { polyline { class: "chart-line-cache", points: segment } }
            for segment in repetition { polyline { class: "chart-line-repetition", points: segment } }
            for (index, point) in points.iter().enumerate() { text { x: "{40.0 + index as f64 / points.len().saturating_sub(1).max(1) as f64 * 740.0}", y: "224", fill: "currentColor", text_anchor: "middle", "{point.cohort}" } }
        }
        div { class: "legend", span { "JSON" } span { "Plain" } span { "Unknown" } span { "Cache" } span { "Repetition" } }
        DataTable {
        thead { tr { th { "Cohort" } th { class: "numeric", "JSON" } th { class: "numeric", "Plain" } th { class: "numeric", "Unknown" } th { class: "numeric", "Cache" } th { class: "numeric", "Repetition" } th { "Gate" } } }
        tbody { for point in points { tr { td { class: "mono", "{point.cohort}" } td { class: "numeric", "{format_ratio(point.json_share)}" } td { class: "numeric", "{format_ratio(point.plain_share)}" }
            td { class: "numeric", "{format_ratio(point.unknown_share)}" } td { class: "numeric", "{format_ratio(point.cache_ratio)}" } td { class: "numeric", "{format_ratio(point.repetition)}" }
            td { Badge { text: stability_label(point.stable), tone: stability_tone(point.stable) } }
        } } }
    } } }
}

#[component]
fn OpportunitiesPage() -> Element {
    let resource = use_resource(|| fetch::<Vec<OpportunitySummary>>("/api/v1/opportunities"));
    rsx! { PageHeader { title: "Optimization Opportunities", subtitle: "Evidence-ranked categories. Candidate priority is not a savings forecast." } div { class: "page", {render_opportunities(&resource.read())} } }
}

fn render_opportunities(state: &Option<Result<Vec<OpportunitySummary>, String>>) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(rows)) => rsx! { DataTable {
            thead { tr { th { "Category" } th { class: "numeric", "Effective presence" } th { class: "numeric", "Repeated tokens" } th { class: "numeric", "Persistence" } th { class: "numeric", "Redundancy" } th { class: "numeric", "Confidence" } th { class: "numeric", "Candidate priority" } } }
            tbody { for row in rows { tr { td { "{humanize(&row.category)}" } td { class: "numeric", "{format_optional_u64(row.effective_presence)}" } td { class: "numeric", "{format_optional_u64(row.repeated_tokens)}" }
                td { class: "numeric", "{format_decimal(row.persistence)}" } td { class: "numeric", "{format_decimal(row.redundancy)}" } td { class: "numeric", "{format_ratio(row.measurement_confidence)}" } td { class: "numeric", "{format_decimal(row.candidate_priority)}" }
            } } }
        } },
    }
}

#[component]
fn CompressionPage() -> Element {
    let resource = use_resource(|| {
        fetch::<Vec<CompressionExperimentSummary>>("/api/v1/compression/experiments")
    });
    rsx! { PageHeader { title: "Compression Lab", subtitle: "Shadow candidate evidence only. No provider request is modified." }
        div { class: "page", {render_compression_experiments(&resource.read())} }
    }
}

fn render_compression_experiments(
    state: &Option<Result<Vec<CompressionExperimentSummary>, String>>,
) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(rows)) if rows.is_empty() => {
            rsx! { Card { title: "Shadow Compression Experiments", EmptyState { title: "No experiments recorded yet.", message: "Enable TRACEPRESS_SHADOW_COMPRESSION=on for a bounded shadow run." } } }
        }
        Some(Ok(rows)) => rsx! {
            Card { title: "Shadow Compression Experiments", DataTable {
                thead { tr { th { "Experiment" } th { class: "numeric", "Candidates" } th { class: "numeric", "Sessions" } th { class: "numeric", "Blocks" } th { "Status" } th { "Integrity" } th { "Runtime SHA" } th { "Started" } } }
                tbody { for row in rows {
                    tr {
                        td { Link { class: "mono", to: Route::CompressionDetailPage { id: row.id.clone() }, "{row.id}" } }
                        td { class: "numeric mono", "{compact_u64(row.candidate_count)}" }
                        td { class: "numeric mono", "{compact_u64(row.session_count)}" }
                        td { class: "numeric mono", "{compact_u64(row.block_count)}" }
                        td { Badge { text: row.status.clone(), tone: compression_status_tone(&row.status) } }
                        td { Badge { text: if compression_quality_healthy(&row.quality) { "healthy" } else { "degraded" }, tone: if compression_quality_healthy(&row.quality) { "success" } else { "danger" } } }
                        td { class: "mono", "{short_identifier(row.runtime_sha.as_deref())}" }
                        td { class: "mono", "{row.started_at}" }
                    }
                } }
            } }
        },
    }
}

#[component]
fn ShadowQualityBanner(quality: tracepress_dashboard_types::CompressionQuality) -> Element {
    let healthy = compression_quality_healthy(&quality);
    rsx! { div { class: if healthy { "quality-banner" } else { "quality-banner degraded" }, role: "status",
        Badge { text: if healthy { "SHADOW INTEGRITY" } else { "SHADOW DEGRADED" }, tone: if healthy { "success" } else { "danger" } }
        div { class: "metric-inline", span { strong { "Forwarding mutations: " } "{quality.forwarding_mutations}" }
            span { strong { "Shadow drops: " } "{quality.shadow_drops}" }
            span { class: "muted", title: "Queue, byte-budget, work-budget, worker-closed and persistence drop counters.", "queue " "{quality.shadow_queue_full_drops}" " · bytes " "{quality.shadow_byte_budget_drops}" " · work " "{quality.shadow_work_budget_drops}" " · closed " "{quality.shadow_worker_closed_drops}" " · persistence " "{quality.shadow_persistence_drops}" }
            span { strong { "Recovery failures: " } "{quality.recovery_failures}" }
            span { strong { "Determinism failures: " } "{quality.determinism_failures}" }
        }
    } }
}

#[component]
fn CompressionDetailPage(id: String) -> Element {
    let detail_id = id.clone();
    let detail = use_resource(move || {
        let url = format!("/api/v1/compression/experiments/{detail_id}");
        async move { fetch::<CompressionExperimentDetail>(&url).await }
    });
    rsx! { PageHeader { title: id, subtitle: "Candidate reduction is local representation evidence, not provider token savings." }
        div { class: "page", {render_compression_detail(&detail.read())} }
    }
}

fn render_compression_detail(
    detail: &Option<Result<CompressionExperimentDetail, String>>,
) -> Element {
    match detail {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(detail)) => rsx! {
            ShadowQualityBanner { quality: detail.summary.quality.clone() }
            div { class: "spacer-top", Card { title: "Candidate comparison", DataTable {
                thead { tr { th { "Candidate" } th { class: "numeric", "Applicable" } th { class: "numeric", "Byte ↓" } th { class: "numeric", "Effective byte ↓" } th { class: "numeric", "Unique byte ↓" } th { class: "numeric", "Est. token ↓" } th { class: "numeric", "Recovery" } th { class: "numeric", "Deterministic" } th { class: "numeric", "P95" } } }
                tbody { for compressor in &detail.compressors { tr {
                    td { class: "mono", "{compressor.compressor}" span { class: "muted", " v{compressor.version}" } }
                    td { class: "numeric", "{format_basis_points(compressor.applicability_basis_points)}" }
                    td { class: "numeric", "{format_basis_points(compressor.byte_reduction_basis_points)}" }
                    td { class: "numeric mono", "{format_optional_u64(compressor.candidate_effective_byte_reduction)}" }
                    td { class: "numeric mono", "{format_optional_u64(compressor.unique_candidate_byte_reduction)}" }
                    td { class: "numeric", "{format_basis_points(compressor.estimated_reduction_basis_points)}" }
                    td { class: "numeric", "{format_basis_points(compressor.recovery_basis_points)}" }
                    td { class: "numeric", "{format_basis_points(compressor.deterministic_basis_points)}" }
                    td { class: "numeric mono", "{format_optional_u64(compressor.processing_p95_us)} µs" }
                } } }
            } } }
            div { class: "grid two spacer-top", CompressionHistogram { title: "Reduction distribution", rows: detail.reduction_histogram.clone(), scope: "Applicable candidates only" }
                CompressionHistogram { title: "Latency distribution", rows: detail.latency_histogram.clone(), scope: "All measured candidates" }
            }
            if !detail.workload_distribution_available { div { class: "spacer-top", Card { title: "Workload distributions", div { class: "compact-notice", strong { "Unavailable. " } "Operational sessions do not yet carry a certified workload mapping; no report label is inferred." } } } }
            CandidateTable { experiment_id: detail.summary.id.clone() }
        },
    }
}

#[component]
fn CompressionHistogram(
    title: String,
    rows: Vec<CompressionHistogramBucket>,
    scope: String,
) -> Element {
    let maximum = rows.iter().map(|row| row.count).max().unwrap_or(1).max(1);
    let observations: u64 = rows.iter().map(|row| row.count).sum();
    rsx! { ChartContainer { title, meta: format!("{scope} · n={observations} · bounded metadata-only distribution"),
        div { class: "bar-list", for row in rows { div { class: "bar-row", span { class: "bar-label mono", "{row.label}" }
            progress { class: "bar-track", max: "{maximum}", value: "{row.count}", aria_label: "{row.label}: {row.count}" }
            span { class: "bar-value mono", "{row.count}" }
        } } }
    } }
}

#[component]
fn CandidateTable(experiment_id: String) -> Element {
    let cursor = use_signal(|| None::<String>);
    let resource = use_resource(move || {
        let cursor_value = cursor.read().clone();
        let experiment_id = experiment_id.clone();
        async move {
            let mut url =
                format!("/api/v1/compression/experiments/{experiment_id}/candidates?limit=50");
            if let Some(value) = cursor_value {
                url.push_str(&format!("&cursor={value}"));
            }
            fetch::<Page<CompressionCandidateSummary>>(&url).await
        }
    });
    rsx! { div { class: "spacer-top", {render_candidate_rows(&resource.read(), cursor)} } }
}

fn render_candidate_rows(
    state: &Option<Result<Page<CompressionCandidateSummary>, String>>,
    cursor: Signal<Option<String>>,
) -> Element {
    match state {
        None => rsx! { Skeleton {} },
        Some(Err(error)) => rsx! { ErrorState { message: error.clone() } },
        Some(Ok(page)) => rsx! { Card { title: "Block-level candidate metadata", DataTable {
            thead { tr { th { "Candidate" } th { "Block" } th { "Type" } th { "Status" } th { class: "numeric", "Input" } th { class: "numeric", "Output" } th { class: "numeric", "Est. input" } th { class: "numeric", "Est. output" } th { "Recovery" } th { "Cache risk" } } }
            tbody { for row in &page.items { tr {
                td { class: "mono", title: row.id.clone(), "{short_candidate_id(&row.id)}" }
                td { class: "mono", "#{row.block_ordinal} {row.block_kind}" div { class: "muted", "{row.origin}" } }
                td { "{optional_text(&row.detected_kind)}" }
                td { Badge { text: row.status.clone(), tone: compression_status_tone(&row.status) } }
                td { class: "numeric mono", "{compact_u64(row.input_bytes)}" }
                td { class: "numeric mono", "{format_optional_u64(row.output_bytes)}" }
                td { class: "numeric mono", "{format_optional_u64(row.input_estimated_tokens)}" }
                td { class: "numeric mono", "{format_optional_u64(row.output_estimated_tokens)}" }
                td { Badge { text: if row.recovery_verified { "verified" } else { "failed/unavailable" }, tone: if row.recovery_verified { "success" } else { "danger" } } }
                td { Badge { text: row.cache_risk.clone(), tone: if row.cache_risk == "high" { "warning" } else { "neutral" } } }
            } } }
        } Pagination { next: page.next_cursor.clone(), cursor } } },
    }
}

#[component]
fn NotFoundPage(route: Vec<String>) -> Element {
    rsx! { div { class: "page", EmptyState { title: "Page not found", message: format!("No Observatory view exists at /{}", route.join("/")) } } }
}

#[component]
fn MetricFromU64(label: String, metric: WireMetric<u64>) -> Element {
    rsx! { Metric { label, value: format_metric_u64(&metric), exact: metric.value.map(|value| value.to_string()), source: source_label(metric.source) } }
}

#[component]
fn MetricFromRatio(label: String, metric: WireMetric<f64>) -> Element {
    rsx! { Metric { label, value: format_metric_ratio(&metric), exact: metric.value.map(|value| format!("{:.6}%", value * 100.0)), source: source_label(metric.source) } }
}

async fn fetch<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let response = Request::get(url)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(format!("API returned HTTP {}", response.status()));
    }
    response
        .json::<T>()
        .await
        .map_err(|error| error.to_string())
}

fn compact_u64(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.2}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.1}K", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

fn compression_quality_healthy(quality: &tracepress_dashboard_types::CompressionQuality) -> bool {
    quality.forwarding_mutations == 0
        && quality.shadow_drops == 0
        && quality.recovery_failures == 0
        && quality.determinism_failures == 0
}

fn short_candidate_id(value: &str) -> String {
    if value.chars().count() <= 18 {
        return value.to_owned();
    }
    let prefix: String = value.chars().take(9).collect();
    let suffix: String = value
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{prefix}…{suffix}")
}

fn format_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "—".to_owned(), compact_u64)
}
fn format_metric_u64(metric: &WireMetric<u64>) -> String {
    format_optional_u64(metric.value)
}
fn format_metric_ratio(metric: &WireMetric<f64>) -> String {
    format_ratio(metric.value)
}
fn format_ratio(value: Option<f64>) -> String {
    value.filter(|number| number.is_finite()).map_or_else(
        || "—".to_owned(),
        |number| format!("{:.1}%", number * 100.0),
    )
}
fn format_decimal(value: Option<f64>) -> String {
    value
        .filter(|number| number.is_finite())
        .map_or_else(|| "—".to_owned(), |number| format!("{number:.2}"))
}
fn strength_class(value: Option<f64>) -> String {
    value.filter(|value| value.is_finite()).map_or_else(
        || "matrix-cell unavailable".to_owned(),
        |value| {
            let normalized = value.clamp(0.0, 1.0);
            let bucket = [0.05, 0.15, 0.25, 0.35, 0.45, 0.55, 0.65, 0.75, 0.85, 0.95]
                .iter()
                .position(|threshold| normalized < *threshold)
                .unwrap_or(10);
            format!("matrix-cell strength-{bucket}")
        },
    )
}
fn format_duration(value: Option<u64>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |us| {
            if us >= 1_000_000 {
                format!("{:.1}s", us as f64 / 1_000_000.0)
            } else {
                format!("{}ms", us / 1_000)
            }
        },
    )
}
fn optional_count(value: u64) -> String {
    if value == 0 {
        "—".to_owned()
    } else {
        value.to_string()
    }
}
fn optional_bool(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "—",
    }
}
fn optional_text(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("—")
}
fn optional_label<'value>(value: &'value Option<String>, fallback: &'value str) -> &'value str {
    value.as_deref().unwrap_or(fallback)
}
fn optional_short_id(value: &Option<String>) -> String {
    value.as_deref().map_or_else(|| "—".to_owned(), short_id)
}
fn join_reasons(values: &[String]) -> String {
    values.join(" · ")
}
fn source_label(source: MetricSource) -> &'static str {
    match source {
        MetricSource::ProviderReported => "Provider reported",
        MetricSource::LocallyEstimated => "Locally estimated",
        MetricSource::Unavailable => "Unavailable",
    }
}
fn short_id(value: &str) -> String {
    if value.chars().count() > 12 {
        format!("{}…", value.chars().take(12).collect::<String>())
    } else {
        value.to_owned()
    }
}
fn short_identifier(value: Option<&str>) -> String {
    value.map_or_else(|| "—".to_owned(), short_id)
}
fn format_basis_points(value: Option<u16>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |basis_points| format!("{:.2}%", f64::from(basis_points) / 100.0),
    )
}
fn humanize(value: &str) -> String {
    value
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + chars.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}
fn status_tone(status: &str) -> String {
    match status {
        "complete" | "completed" | "healthy" => "success",
        "running" | "partial" => "warning",
        "failed" | "dropped" | "malformed" => "danger",
        _ => "neutral",
    }
    .to_owned()
}
fn compression_status_tone(status: &str) -> String {
    match status {
        "applicable" | "completed" => "success",
        "running" | "resource_limit" => "warning",
        "recovery_failed" | "internal_error" | "failed" => "danger",
        _ => "neutral",
    }
    .to_owned()
}
fn baseline_tone(status: &str) -> String {
    match status {
        "converged" | "valid" => "success",
        "legacy_uncertified" => "warning",
        _ => "neutral",
    }
    .to_owned()
}
fn stability_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Stable",
        Some(false) => "Unstable",
        None => "Unavailable",
    }
}
fn stability_tone(value: Option<bool>) -> String {
    match value {
        Some(true) => "success",
        Some(false) => "warning",
        None => "neutral",
    }
    .to_owned()
}

fn polyline_segments<F>(points: &[ContextGrowthPoint], max: f64, value: F) -> Vec<String>
where
    F: Fn(&ContextGrowthPoint) -> Option<u64>,
{
    let divisor = points.len().saturating_sub(1).max(1) as f64;
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for (index, point) in points.iter().enumerate() {
        match value(point) {
            Some(sample) => {
                let x = 40.0 + index as f64 / divisor * 740.0;
                let y = 205.0 - sample as f64 / max * 170.0;
                current.push(format!("{x:.1},{y:.1}"));
            }
            None if !current.is_empty() => segments.push(std::mem::take(&mut current).join(" ")),
            None => {}
        }
    }
    if !current.is_empty() {
        segments.push(current.join(" "));
    }
    segments
}

fn convergence_segments<F>(
    points: &[tracepress_dashboard_types::ConvergencePoint],
    value: F,
) -> Vec<String>
where
    F: Fn(&tracepress_dashboard_types::ConvergencePoint) -> Option<f64>,
{
    let divisor = points.len().saturating_sub(1).max(1) as f64;
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for (index, point) in points.iter().enumerate() {
        match value(point).filter(|sample| sample.is_finite()) {
            Some(sample) => {
                let x = 40.0 + index as f64 / divisor * 740.0;
                let y = 205.0 - sample.clamp(0.0, 1.0) * 170.0;
                current.push(format!("{x:.1},{y:.1}"));
            }
            None => {
                if !current.is_empty() {
                    segments.push(current.join(" "));
                    current.clear();
                }
            }
        }
    }
    if !current.is_empty() {
        segments.push(current.join(" "));
    }
    segments
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;

    use super::{EmptyState, ErrorState, FilterBar, Skeleton, compact_u64, format_ratio, short_id};
    #[test]
    fn formats_dense_numbers_and_missing_values() {
        assert_eq!(compact_u64(7_911_876), "7.91M");
        assert_eq!(format_ratio(None), "—");
        assert_eq!(short_id("0123456789abcdef"), "0123456789ab…");
    }

    #[test]
    fn loading_empty_error_and_filter_states_render() {
        let loading = dioxus_ssr::render_element(rsx! { Skeleton {} });
        let empty = dioxus_ssr::render_element(rsx! {
            EmptyState { title: "No sessions", message: "No operational sessions." }
        });
        let error = dioxus_ssr::render_element(rsx! {
            ErrorState { message: "Typed API failure" }
        });
        let filter = dioxus_ssr::render_element(rsx! {
            FilterBar { label { "Status" } }
        });
        assert!(loading.contains("Loading"));
        assert!(empty.contains("No sessions"));
        assert!(error.contains("Typed API failure"));
        assert!(filter.contains("filter-bar"));
    }

    #[test]
    fn csp_compatible_markup_has_no_inline_style_attributes() {
        let source = include_str!("main.rs");
        let inline_style_attribute = ["style", ":"].concat();
        assert!(
            !source.contains(&inline_style_attribute),
            "Dioxus style attributes are blocked by the Observatory CSP"
        );
    }
}
