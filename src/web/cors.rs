use axum::http::{HeaderValue, Method};
use tower_http::cors::{Any, CorsLayer};

pub fn cors_layer(allow_origins: &[&str]) -> CorsLayer {
    let origins = allow_origins.iter().map(|x| x.parse::<HeaderValue>().unwrap()).collect::<Vec<HeaderValue>>();
    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
}