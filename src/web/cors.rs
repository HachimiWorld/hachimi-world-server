use std::time::Duration;
use axum::http::{HeaderValue, Method};
use tower_http::cors::{AllowOrigin, Any, CorsLayer, MaxAge};

pub fn cors_layer(allow_origins: &[&str]) -> CorsLayer {
    let has_any = allow_origins.contains(&"*");
    let origins = allow_origins.iter().map(|x| x.parse::<HeaderValue>().unwrap()).collect::<Vec<HeaderValue>>();

    CorsLayer::new()
        .allow_origin(if has_any { AllowOrigin::any() } else { AllowOrigin::list(origins) })
        .allow_methods([Method::GET, Method::POST])
        .allow_headers(Any)
        .max_age(MaxAge::exact(Duration::from_secs(86400)))
}