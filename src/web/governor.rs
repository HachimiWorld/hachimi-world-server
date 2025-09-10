use axum::http::{HeaderName, Request};
use governor::middleware::NoOpMiddleware;
use std::net::{IpAddr, SocketAddr};
use axum_extra::headers::HeaderMapExt;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};
use tracing::error;

pub fn governor_layer<RespBody>() -> GovernorLayer<RealIPExtractor, NoOpMiddleware, RespBody> {

    // Allow bursts with up to five requests per IP address
    // and replenishes one element every two seconds
    let governor_conf = GovernorConfigBuilder::default()
        .key_extractor(RealIPExtractor)
        .finish().unwrap();
    GovernorLayer::new(governor_conf)
}

#[derive(Clone, Debug)]
pub struct RealIPExtractor;

impl KeyExtractor for RealIPExtractor {
    type Key = IpAddr;
    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        let headers = req.headers();
        headers.get(HeaderName::from_static("x-real-ip"))
            .and_then(|hv| hv.to_str().ok())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .ok_or_else(|| {
                error!("Failed to extract real IP from headers");
                GovernorError::UnableToExtractKey
            })
    }
}