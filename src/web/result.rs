use axum::http::StatusCode;
use axum::Json;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};

#[macro_export]
macro_rules! common {
    ($code:expr, $($arg:tt)*) => {
        crate::web::result::WebError::common($code, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! err {
    ($code:expr, $($arg:tt)*) => {
        return Err($crate::web::result::WebError::common($code, &format!($($arg)*)))
    };
}

#[macro_export]
macro_rules! ok {
    ($data:expr) => {
        return Ok(axum::Json($crate::web::result::WebResponse::ok($data)))
    };
}

/// This is used as the return type for web handlers. Then handlers can use `?` grammar for error handling.
///
/// When a handler returns `OK`, it will return status 200 OK.
///
/// When a `WebError` returns, the status is decided by the error type.
/// - `Internal` means 500 without response body. This is the default type when an error is returned by `?`.
/// - `BusinessError` means 200 with an JSON error message (Default to `CommonError` type). This type should be instantiated manually (e.g., by the `err!` macro)
pub type WebResult<T, E = CommonError> = Result<Json<WebResponse<T>>, WebError<E>>;

/// This is used to represent the unified JSON results.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WebResponse<T> {
    pub ok: bool,
    pub data: T,
}

// Use anyhow, define error and enable '?'
// For a simplified example of using anyhow in axum check /examples/anyhow-error-response
#[derive(Debug)]
pub enum WebError<E: Serialize> {
    Business(E),
    Internal(anyhow::Error),
}

impl<E, F> From<E> for WebError<F>
where
    E: Into<anyhow::Error>,
    F: Serialize,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonError {
    pub code: String,
    pub msg: String,
}

impl <T> WebResponse<T> {
    pub fn ok(data: T) -> Self {
        WebResponse {
            ok: true,
            data,
        }
    }

    pub fn err(data: T) -> Self {
        WebResponse {
            ok: false,
            data,
        }
    }
}

impl WebError<CommonError> {
    pub fn common(code: &str, msg: &str) -> Self {
        WebError::Business(CommonError {
            code: code.to_string(),
            msg: msg.to_string(),
        })
    }
}

impl<E: Serialize> IntoResponse for WebError<E> {
    fn into_response(self) -> axum::response::Response {
        match self {
            WebError::Business(resp) => {
                Json(WebResponse::err(resp)).into_response()
            }
            WebError::Internal(err) => {
                tracing::error!("Internal error occurs in handlers. \n{:?}", err);
                // 可以选择不使用 `err.to_string()`
                // let error_message = format!("Something went wrong: {}", err.to_string());
                let error_message = "Something went wrong";
                (StatusCode::INTERNAL_SERVER_ERROR, error_message).into_response()
            }
        }
    }
}