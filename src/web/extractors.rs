use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use crate::web::result::WebError;

#[derive(Debug, Clone)]
pub struct XRealIP(pub String);

impl<S> FromRequestParts<S> for XRealIP
where
    S: Send + Sync,
{
    type Rejection = WebError<()>;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let value = parts
            .headers
            .get("X-Real-IP")
            .or_else(|| parts.headers.get("CF-Connecting-IP")) // TODO[security]: Is it safe?
            .ok_or_else(|| WebError::Internal(anyhow::anyhow!("X-Real-IP header not found")))?;

        Ok(XRealIP(value.to_str()?.to_string()))
    }
}