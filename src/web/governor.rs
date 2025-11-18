use axum::http::{HeaderName, Request};
use governor::middleware::NoOpMiddleware;
use std::net::{IpAddr, SocketAddr};
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};
use tracing::error;

pub fn governor_layer<RespBody>() -> GovernorLayer<RealIPExtractor, NoOpMiddleware, RespBody> {
    let governor_conf = GovernorConfigBuilder::default()
        .burst_size(16)
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
        let real_ip = headers.get(HeaderName::from_static("x-real-ip"))
            .and_then(|hv| hv.to_str().ok())
            .and_then(|s| s.parse::<IpAddr>().ok());
        let peer_ip = req.extensions().get::<axum::extract::ConnectInfo<SocketAddr>>().map(|sa| sa.ip());
        real_ip.or(peer_ip).ok_or_else(|| {
            error!("Failed to extract real IP from headers");
            GovernorError::UnableToExtractKey
        })
    }
}