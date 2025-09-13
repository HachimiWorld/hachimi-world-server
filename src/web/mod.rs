use std::net::SocketAddr;
use crate::web::state::AppState;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::routing::get;
use axum::{Router, ServiceExt};
use serde::Deserialize;
use tokio::net::ToSocketAddrs;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

pub mod routes;
pub mod state;

mod jwt;
pub mod result;
mod web_metrics;
mod extractors;
mod governor;
mod request_id;
mod cors;

#[derive(Deserialize)]
pub struct ServerCfg {
    pub listen: String,
    pub metrics_listen: String,
    pub jwt_secret: String,
    pub allow_origins: Vec<String>,
    pub publish_version_token: String
}

pub async fn run_web_app(
    cfg: ServerCfg,
    app_state: AppState,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    jwt::initialize_jwt_key(jwt::Keys::new(cfg.jwt_secret.as_bytes()));
    jwt::initialize_version_token(cfg.publish_version_token);

    let allow_origins = cfg.allow_origins.iter().map(|x| x.as_str()).collect::<Vec<&str>>();
    let (_main_server, _metrics_server) = tokio::join!(
        start_main_server(app_state, cfg.listen, &allow_origins, cancel_token.clone()),
        web_metrics::start_metrics_server(cfg.metrics_listen, cancel_token)
    );

    info!("Web server stopped");
    Ok(())
}


async fn start_main_server(
    app_state: AppState,
    addr: impl ToSocketAddrs,
    allow_origins: &[&str],
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("HTTP Server started at {}", listener.local_addr()?);
    
    let app = Router::new()
        .nest("/api", routes::router())
        .route("/health", get(health))
        .with_state(app_state)
        .route_layer(axum::middleware::from_fn(web_metrics::track_metrics))
        .layer(cors::cors_layer(allow_origins))
        .layer(request_id::request_id_layer())
        .layer(governor::governor_layer());

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            cancel_token.cancelled().await;
            info!("Shutting down web server...");
        })
        .await?;
    Ok(())
}

async fn health() -> StatusCode {
    // TODO[refactor]: Check more services
    StatusCode::OK
}
