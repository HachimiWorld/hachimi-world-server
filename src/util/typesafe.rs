//! Client for TypeSafe's System One API, which runs the Jev models: it asks typed questions about
//! a `state` and returns typed answers. See https://docs.typesafe.ai/api.

use async_trait::async_trait;
use mockall::automock;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;

/// @since 261016
#[derive(Debug, Clone, Deserialize)]
pub struct TypeSafeCfg {
    pub api_key: String,
    /// Such as `https://api.typesafe.ai`.
    pub base_url: String,
    pub timeout_ms: u64,
}

/// The API answered with an error status. Other failures, such as timeouts and network errors,
/// come as other errors.
/// @since 261017
#[derive(Debug, thiserror::Error)]
#[error("TypeSafe returned {status}: {body}")]
pub struct TypeSafeStatusError {
    pub status: reqwest::StatusCode,
    /// Shortened; it can echo the request.
    pub body: String,
}

impl TypeSafeStatusError {
    /// Whether retrying can't help because the request or the config is wrong: a bad key, an
    /// unknown model, malformed questions or a wrong `base_url`. Timeouts, rate limits and server
    /// errors are passing.
    pub fn is_config_error(&self) -> bool {
        self.status.is_client_error()
            && self.status != reqwest::StatusCode::REQUEST_TIMEOUT
            && self.status != reqwest::StatusCode::TOO_MANY_REQUESTS
    }
}

/// @since 261016
#[async_trait]
#[automock]
pub trait TypeSafeClient: Send + Sync {
    /// Asks `questions`, the API's `questions` map, about the text `state` with `model`.
    async fn system_one(&self, model: &str, state: &str, questions: &serde_json::Value) -> anyhow::Result<SystemOneResponse>;
}

/// @since 261016
#[derive(Debug, Clone, Deserialize)]
pub struct SystemOneResponse {
    /// The model that answered, such as `jev-1.13.0`.
    pub model: String,
    /// Question id -> answer, kept as returned.
    pub answers: serde_json::Value,
    pub usage: Usage,
}

/// @since 261016
#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    pub input_tokens: i64,
}

impl SystemOneResponse {
    /// The probability of yes for a noul question; `None` if `id` is missing or isn't a noul.
    pub fn noul(&self, id: &str) -> Option<f64> {
        let answer = self.answers.get(id)?;
        if answer.get("type")?.as_str()? != "noul" {
            return None;
        }
        answer.get("noul")?.as_f64()
    }
}

#[async_trait]
impl<T: TypeSafeClient + ?Sized> TypeSafeClient for Arc<T> {
    async fn system_one(&self, model: &str, state: &str, questions: &serde_json::Value) -> anyhow::Result<SystemOneResponse> {
        (**self).system_one(model, state, questions).await
    }
}

pub struct TypeSafeClientImpl {
    client: reqwest::Client,
    api_key: String,
    endpoint: String,
}

impl TypeSafeClientImpl {
    pub fn new(cfg: &TypeSafeCfg) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .build()?;
        Ok(Self {
            client,
            api_key: cfg.api_key.clone(),
            endpoint: format!("{}/v1/systemone", cfg.base_url.trim_end_matches('/')),
        })
    }
}

#[async_trait]
impl TypeSafeClient for TypeSafeClientImpl {
    async fn system_one(&self, model: &str, state: &str, questions: &serde_json::Value) -> anyhow::Result<SystemOneResponse> {
        // No retries: callers that make a user wait would rather go on without an answer, and
        // background callers retry on their own schedule.
        let resp = self.client.post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({ "model": model, "state": state, "questions": questions }))
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            // The body can echo the request; keep the log short.
            let body: String = body.chars().take(300).collect();
            return Err(TypeSafeStatusError { status, body }.into());
        }
        Ok(serde_json::from_str(&body)?)
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemOneResponse, TypeSafeStatusError};
    use reqwest::StatusCode;

    #[test]
    fn tells_config_errors_from_passing_ones() {
        let is_config = |status| TypeSafeStatusError { status, body: String::new() }.is_config_error();
        assert!(is_config(StatusCode::UNAUTHORIZED));
        assert!(is_config(StatusCode::FORBIDDEN));
        assert!(is_config(StatusCode::NOT_FOUND));
        assert!(is_config(StatusCode::UNPROCESSABLE_ENTITY));
        assert!(!is_config(StatusCode::REQUEST_TIMEOUT));
        assert!(!is_config(StatusCode::TOO_MANY_REQUESTS));
        assert!(!is_config(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_config(StatusCode::from_u16(529).unwrap()));
    }

    #[test]
    fn parses_a_response_and_reads_nouls() {
        let resp: SystemOneResponse = serde_json::from_str(r#"{
            "model": "jev-1.13.0",
            "answers": {
                "politics": { "type": "noul", "noul": 0.95 },
                "tone": { "type": "choice", "choice": "meme", "probabilities": { "meme": 1.0 }, "confidence": 1.0 }
            },
            "usage": { "input_tokens": 296, "output_tokens": 20 }
        }"#).unwrap();

        assert_eq!(resp.model, "jev-1.13.0");
        assert_eq!(resp.usage.input_tokens, 296);
        assert_eq!(resp.noul("politics"), Some(0.95));
        assert_eq!(resp.noul("tone"), None);
        assert_eq!(resp.noul("missing"), None);
    }
}
