use crate::web::state::AppState;
use axum::http::{HeaderValue, Method, StatusCode};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tokio::net::ToSocketAddrs;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tracing::{info};

pub mod routes;
pub mod state;

mod jwt;
pub mod result;
mod web_metrics;
mod extractors;
mod governor;
mod request_id;

#[derive(Deserialize)]
pub struct ServerCfg {
    pub listen: String,
    pub metrics_listen: String,
    pub jwt_secret: String,
    pub allow_origin: String,
}

pub async fn run_web_app(
    cfg: ServerCfg,
    app_state: AppState,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    jwt::initialize_jwt_key(jwt::Keys::new(cfg.jwt_secret.as_bytes()));
    
    let (_main_server, _metrics_server) = tokio::join!(
        start_main_server(app_state, cfg.listen, cfg.allow_origin, cancel_token.clone()),
        web_metrics::start_metrics_server(cfg.metrics_listen, cancel_token)
    );

    info!("Web server stopped");
    Ok(())
}


async fn start_main_server(
    app_state: AppState,
    addr: impl ToSocketAddrs,
    allow_origin: String,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("HTTP Server started at {}", listener.local_addr()?);
    
    let app = Router::new()
        .nest("/api", routes::router())
        .route("/health", get(health))
        .with_state(app_state)
        .route_layer(axum::middleware::from_fn(web_metrics::track_metrics))
        .layer(CorsLayer::new()
            .allow_origin(allow_origin.parse::<HeaderValue>()?)
            .allow_methods([Method::GET, Method::POST])
        )
        .layer(request_id::request_id_layer())
        .layer(governor::governor_layer());

    axum::serve(listener, app)
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
