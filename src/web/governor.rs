use crate::web::jwt;
use axum::http::{header, HeaderName, Request};
use governor::middleware::NoOpMiddleware;
use serde::Deserialize;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;
use tower_governor::governor::GovernorConfigBuilder;
use tower_governor::key_extractor::KeyExtractor;
use tower_governor::{GovernorError, GovernorLayer};
use tracing::error;

/// Global limit on `/api`. It only stops a client that loops by mistake; costly endpoints
/// keep their own per-user limits.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(default)]
pub struct RateLimitCfg {
    /// Requests refilled per second for each key
    pub per_second: u32,
    /// Requests a key may send at once before it is limited
    pub burst_size: u32,
}

impl Default for RateLimitCfg {
    fn default() -> Self {
        Self { per_second: 10, burst_size: 60 }
    }
}

pub fn governor_layer<RespBody>(
    cfg: RateLimitCfg,
) -> GovernorLayer<UserOrIpExtractor, NoOpMiddleware, RespBody> {
    let governor_conf = GovernorConfigBuilder::default()
        .period(Duration::from_secs(1) / cfg.per_second.max(1))
        .burst_size(cfg.burst_size)
        .key_extractor(UserOrIpExtractor)
        .finish().unwrap();

    // The limiter keeps one entry per key forever unless it is cleaned up
    let limiter = governor_conf.limiter().clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            limiter.retain_recent();
        }
    });

    GovernorLayer::new(governor_conf)
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum RateLimitKey {
    User(i64),
    Ip(IpAddr),
}

/// Signed-in requests are limited per user, so users behind one NAT don't share a quota.
/// Anonymous requests and requests with an invalid token fall back to the client IP.
#[derive(Clone, Debug)]
pub struct UserOrIpExtractor;

impl KeyExtractor for UserOrIpExtractor {
    type Key = RateLimitKey;
    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, GovernorError> {
        if let Some(uid) = user_id(req) {
            return Ok(RateLimitKey::User(uid));
        }
        client_ip(req).map(RateLimitKey::Ip).ok_or_else(|| {
            error!("Failed to extract real IP from headers");
            GovernorError::UnableToExtractKey
        })
    }
}

fn user_id<T>(req: &Request<T>) -> Option<i64> {
    let token = req.headers().get(header::AUTHORIZATION)?
        .to_str().ok()?
        .strip_prefix("Bearer ")?;
    jwt::decode_and_validate_access_token(token).ok()?.sub.parse().ok()
}

fn client_ip<T>(req: &Request<T>) -> Option<IpAddr> {
    let real_ip = req.headers().get(HeaderName::from_static("x-real-ip"))
        .and_then(|hv| hv.to_str().ok())
        .and_then(|s| s.parse::<IpAddr>().ok());
    let peer_ip = || req.extensions().get::<axum::extract::ConnectInfo<SocketAddr>>().map(|sa| sa.0.ip());
    real_ip.or_else(peer_ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn init_keys() {
        jwt::initialize_jwt_key(jwt::Keys::new(b"test-secret"));
    }

    fn request(auth: Option<&str>, real_ip: &str) -> Request<Body> {
        let mut builder = Request::builder().header("x-real-ip", real_ip);
        if let Some(auth) = auth {
            builder = builder.header(header::AUTHORIZATION, auth);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn signed_in_requests_are_keyed_by_user() {
        init_keys();
        let exp = chrono::Utc::now().timestamp() + 300;
        let token = jwt::generate_access_token("42", exp);
        let key = UserOrIpExtractor.extract(&request(Some(&format!("Bearer {token}")), "1.2.3.4")).unwrap();
        assert_eq!(key, RateLimitKey::User(42));
    }

    #[test]
    fn anonymous_and_invalid_tokens_fall_back_to_ip() {
        init_keys();
        let ip: IpAddr = "1.2.3.4".parse().unwrap();
        assert_eq!(UserOrIpExtractor.extract(&request(None, "1.2.3.4")).unwrap(), RateLimitKey::Ip(ip));
        assert_eq!(UserOrIpExtractor.extract(&request(Some("Bearer not-a-jwt"), "1.2.3.4")).unwrap(), RateLimitKey::Ip(ip));
        let expired = jwt::generate_access_token("42", chrono::Utc::now().timestamp() - 300);
        assert_eq!(UserOrIpExtractor.extract(&request(Some(&format!("Bearer {expired}")), "1.2.3.4")).unwrap(), RateLimitKey::Ip(ip));
    }
}
