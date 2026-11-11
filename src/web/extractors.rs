use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::headers::UserAgent;
use axum_extra::typed_header::TypedHeaderRejection;
use axum_extra::TypedHeader;
use std::net::SocketAddr;
use crate::web::result::WebError;

#[derive(Debug, Clone)]
pub struct XRealIP(pub String);

/// Read from headers the proxy sets, not something clients send.
impl aide::OperationInput for XRealIP {}

impl<S> FromRequestParts<S> for XRealIP
where
    S: Send + Sync,
{
    type Rejection = WebError<()>;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("X-Real-IP")
            .or_else(|| parts.headers.get("CF-Connecting-IP")); // TODO[security]: Is it safe?
        if let Some(value) = header {
            return Ok(XRealIP(value.to_str()?.to_string()));
        }

        // No proxy in front, e.g. a local server: use the connection's address
        let peer_ip = parts.extensions.get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip().to_string())
            .ok_or_else(|| WebError::Internal(anyhow::anyhow!("X-Real-IP header not found")))?;
        Ok(XRealIP(peer_ip))
    }
}

/// The `User-Agent` header, which logins record with the device. Wraps axum-extra's typed header
/// so the OpenAPI document can take it: aide implements its traits for an older axum-extra.
#[derive(Debug, Clone)]
pub struct ClientUserAgent(pub UserAgent);

impl<S> FromRequestParts<S> for ClientUserAgent
where
    S: Send + Sync,
{
    type Rejection = TypedHeaderRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let TypedHeader(ua) = TypedHeader::<UserAgent>::from_request_parts(parts, state).await?;
        Ok(ClientUserAgent(ua))
    }
}

/// Every client sends one, so it isn't listed as a parameter.
impl aide::OperationInput for ClientUserAgent {}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    async fn extract(req: Request<()>) -> Option<String> {
        let (mut parts, _) = req.into_parts();
        XRealIP::from_request_parts(&mut parts, &()).await.ok().map(|XRealIP(ip)| ip)
    }

    #[tokio::test]
    async fn prefers_header_over_connection() {
        let mut req = Request::builder().header("X-Real-IP", "1.2.3.4").body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))));
        assert_eq!(extract(req).await.as_deref(), Some("1.2.3.4"));
    }

    #[tokio::test]
    async fn falls_back_to_connection_ip() {
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 8080))));
        assert_eq!(extract(req).await.as_deref(), Some("127.0.0.1"));
    }

    #[tokio::test]
    async fn fails_without_header_or_connection() {
        assert_eq!(extract(Request::builder().body(()).unwrap()).await, None);
    }
}
