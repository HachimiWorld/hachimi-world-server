use crate::web::state::AppState;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use tokio::net::ToSocketAddrs;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub mod routes;
pub mod state;

mod jwt;
pub mod result;
mod web_metrics;
mod extractors;

pub async fn run_web_app(
    jwt_secret: String,
    app_state: AppState,
    addr: impl ToSocketAddrs,
    metrics_addr: impl ToSocketAddrs,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    jwt::initialize_jwt_key(jwt::Keys::new(jwt_secret.as_bytes()));
    
    let (_main_server, _metrics_server) = tokio::join!(
        start_main_server(app_state, addr, cancel_token.clone()),
        web_metrics::start_metrics_server(metrics_addr, cancel_token)
    );

    info!("Web server stopped");
    Ok(())
}

async fn start_main_server(
    app_state: AppState,
    addr: impl ToSocketAddrs,
    cancel_token: CancellationToken,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("HTTP Server started at {}", listener.local_addr()?);

    let app = Router::new()
        .nest("/api", routes::router())
        .route("/health", get(health))
        .with_state(app_state)
        .route_layer(axum::middleware::from_fn(web_metrics::track_metrics));

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
