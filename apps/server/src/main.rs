//! Raydium × Ifx web server — quote, build, simulate API + static UI.

mod api;
mod amount;
mod token;

use anyhow::Context;
use axum::{
    routing::{get, post},
    Router,
};
use ifx_raydium_config::AppConfig;
use solana_commitment_config::CommitmentConfig;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub commitment: CommitmentConfig,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "ifx_raydium_server=info,ifx_raydium_planner=info,ifx_raydium=info,tower_http=info,warn",
        )
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config.toml".to_string());
    let config = Arc::new(
        AppConfig::load(&config_path)
            .with_context(|| format!("load config from {config_path}"))?,
    );

    let commitment = match config.solana.commitment.as_str() {
        "processed" => CommitmentConfig::processed(),
        "finalized" => CommitmentConfig::finalized(),
        _ => CommitmentConfig::confirmed(),
    };

    let state = AppState {
        config: config.clone(),
        commitment,
    };

    let public_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/public");
    let api = Router::new()
        .route("/api/health", get(api::health))
        .route("/api/config/public", get(api::public_config))
        .route("/api/quote", post(api::quote))
        .route("/api/tx/build", post(api::build_tx))
        .route("/api/tx/simulate", post(api::simulate_tx))
        .route("/api/token/info", post(api::token_info))
        .with_state(state.clone());

    let app = Router::new()
        .merge(api)
        .fallback_service(ServeDir::new(public_dir))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from((
        config.server.host.parse::<std::net::IpAddr>()?,
        config.server.port,
    ));
    tracing::info!("listening on http://{addr}");
    if let Some(proxy) = config.network.effective_http_proxy() {
        tracing::info!(
            proxy = %mask_proxy(&proxy),
            timeout_secs = config.network.raydium_api_timeout_secs,
            "Raydium API network settings"
        );
    } else {
        tracing::info!(
            timeout_secs = config.network.raydium_api_timeout_secs,
            "Raydium API: no proxy configured (set [network].http_proxy or HTTPS_PROXY env)"
        );
    }
    tracing::info!(
        "log filter: set RUST_LOG=debug for verbose routing/quote (default: ifx_raydium*=info)"
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn mask_proxy(url: &str) -> String {
    if let Some(at) = url.rfind('@') {
        format!("***@{}", &url[at + 1..])
    } else {
        url.to_string()
    }
}
