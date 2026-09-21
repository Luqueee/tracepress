//! Local Observatory startup and static-asset discovery.

use super::*;

pub(super) async fn dashboard(config: &Config, options: DashboardOptions) -> Result<(), String> {
    let fixture_database = options
        .fixture
        .then(|| tracepress_dashboard_api::fixture_database(options.large_fixture))
        .transpose()
        .map_err(|error| error.to_string())?;
    let database_path = fixture_database.as_ref().map_or_else(
        || config.database.clone(),
        |database| database.path().to_path_buf(),
    );
    let reports_path = std::env::var_os("TRACEPRESS_REPORTS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports"));
    let assets_path = std::env::var_os("TRACEPRESS_DASHBOARD_ASSETS")
        .map(PathBuf::from)
        .or_else(discover_dashboard_assets);
    let bind = std::net::SocketAddr::from(([127, 0, 0, 1], options.port));
    println!("Tracepress Observatory");
    println!("http://{bind}");
    let mut dashboard_config =
        tracepress_dashboard_api::DashboardConfig::local(database_path, reports_path);
    dashboard_config.bind = bind;
    dashboard_config.assets_path = assets_path;
    tracepress_dashboard_api::serve(dashboard_config)
        .await
        .map_err(|error| error.to_string())
}

pub(super) fn discover_dashboard_assets() -> Option<PathBuf> {
    [
        PathBuf::from("target/dx/tracepress-dashboard/release/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-dashboard/release/web/public",
        ),
        PathBuf::from("target/dx/tracepress-dashboard/debug/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-dashboard/debug/web/public",
        ),
        PathBuf::from("target/dx/tracepress-observatory/release/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-observatory/release/web/public",
        ),
        PathBuf::from("target/dx/tracepress-observatory/debug/web/public"),
        PathBuf::from(
            "crates/tracepress-dashboard/target/dx/tracepress-observatory/debug/web/public",
        ),
    ]
    .into_iter()
    .find(|path| path.join("index.html").is_file())
}
