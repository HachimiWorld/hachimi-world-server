use crate::web::state::AppState;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, RequestPartsExt};
use axum_extra::headers::authorization::Bearer;
use axum_extra::headers::Authorization;
use axum_extra::TypedHeader;
use jsonwebtoken::{encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::{LazyLock, OnceLock};
use uuid::Uuid;

static JWT_KEYS: OnceLock<Keys> = OnceLock::new();

pub fn initialize_jwt_key(keys: Keys) {
    match JWT_KEYS.set(keys) {
        Ok(_) => {}
        Err(_) => {
            panic!("JWT keys already initialized");
        }
    };
}

pub fn generate_access_token(uid: &str, exp: i64) -> String {
    let claims = Claims {
        sub: uid.to_string(),
        iss: "hachimi-world".to_string(),
        iat: chrono::Utc::now().timestamp(),
        exp: exp,
        jti: Uuid::new_v4().to_string(),
    };
    encode(&Header::default(), &claims, &JWT_KEYS.get().unwrap().encoding).unwrap()
}

pub fn generate_refresh_token(uid: &str) -> (String, RefreshTokenClaims) {
    let claims = RefreshTokenClaims {
        r#type: "refresh_token".to_string(),
        uid: uid.to_string(),
        iss: "hachimi-world".to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::days(365)).timestamp() as usize,
        jti: Uuid::new_v4().to_string(),
    };
    let encoded = encode(&Header::default(), &claims, &JWT_KEYS.get().unwrap().encoding).unwrap();
    (encoded, claims)
}


#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshTokenClaims {
    pub r#type: String,
    pub uid: String,
    pub iss: String,
    pub exp: usize,
    pub jti: String,
}


pub fn decode_and_validate_refresh_token(token: &str) -> anyhow::Result<RefreshTokenClaims> {
    let r = jsonwebtoken::decode::<RefreshTokenClaims>(
        token,
        &JWT_KEYS.get().unwrap().decoding,
        &Validation::default(),
    )?;
    Ok(r.claims)
}

pub struct Keys {
    encoding: EncodingKey,
    decoding: DecodingKey,
}

impl Keys {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    pub jti: String,
}

impl Claims {
    pub fn uid(&self) -> i64 {
        self.sub.parse().unwrap()
    }
}

impl FromRequestParts<AppState> for Claims {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Extract the token from the authorization header
        let TypedHeader(Authorization(bearer)) = parts
            .extract::<TypedHeader<Authorization<Bearer>>>()
            .await
            .map_err(|_| AuthError::MissingCredentials)?;
        // Decode the user data
        let token_data = jsonwebtoken::decode::<Claims>(
            bearer.token(),
            &JWT_KEYS.get().unwrap().decoding,
            &Validation::default(),
        )
        .map_err(|_| AuthError::InvalidToken)?;

        Ok(token_data.claims)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AdminClaims {
    pub sub: String,
    pub iss: String,
    pub exp: usize,
    pub jti: String,
}

#[derive(Debug)]
pub enum AuthError {
    WrongCredentials,
    MissingCredentials,
    TokenCreation,
    InvalidToken,
}

// 未认证返回 UNAUTHORIZED
impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            AuthError::WrongCredentials => (StatusCode::UNAUTHORIZED, "Wrong credentials"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token"),
            AuthError::MissingCredentials => (StatusCode::UNAUTHORIZED, "Missing credentials"),
            AuthError::TokenCreation => (StatusCode::INTERNAL_SERVER_ERROR, "Token creation error"),
        };
        let body = Json(json!({
            "error": error_message,
        }));
        (status, body).into_response()
    }
}

#[cfg(test)]
mod test {}
