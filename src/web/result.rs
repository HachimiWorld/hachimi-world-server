//! This module defines the unified response structure for web handlers, including success responses and error responses.
//!
//! ## Handler
//!
//! ### Return type for handler
//!
//! For all axum handlers, we use `Result<Json<WebResponse<T>>, WebError<E>>>` as the return type, where `T` is the success data type, and `E` is the error data type.
//!
//! ```rust
//! async fn greet_handler() -> Result<Json<WebResponse<String>>, WebError<CommonError>> {
//!     return Ok(Json(WebResponse::ok(format!("Hello, world!"))));
//! }
//! ```
//!
//! For convenience, we define a type alias `WebResult<T, E = CommonError>` for it. Where `CommonError` is the most common error data type used in business errors.
//!
//! ```rust
//! async fn greet_handler() -> WebResult<String> {
//!     return Ok(Json(WebResponse::ok(format!("Hello, world!"))));
//! }
//! ```
//!
//! ### Return a success response
//!
//! We can return `Ok(Json(WebResponse::ok(data)))` to indicate a successful response.
//!
//!
//! ```rust
//! async fn greet_handler() -> WebResult<String> {
//!     return Ok(Json(WebResponse::ok(format!("Hello, world!"))));
//! }
//! ```
//!
//! For convenience, we can use macro `ok!(data)` to return a success response, which is equivalent to `return Ok(Json(WebResponse::ok(data)))`
//!
//! ```rust
//! async fn greet_handler() -> WebResult<String> {
//!    ok!(format!("Hello, world!"))
//! }
//! ```
//!
//! ### Return a error response
//!
//! Error responses are separated to Business errors and Internal errors.
//!
//! Business errors are known errors for business logics, for example: user not found, permission denied, etc... They are represented as `WebError::Business<E>` type, and will be converted to a `WebResponse::err` JSON response with HTTP 200 status code.
//!
//! There is also a convenience macro `err!(code, msg)` to return a business error, which is equivalent to `return Err(WebError::Business(CommonError { code: code.to_string(), msg: msg.to_string() }))`
//!
//! ```rust
//! use hachimi_world_server::web::result::WebError;
//!
//!  async fn greet_handler(uid: Query<i32>) -> WebResult<String> {
//!     if uid == 255 {
//!         ok!(format!("Hello, world!"));
//!     } else {
//!         return Err(WebError::Business(CommonError {
//!            code: "user_not_found".to_string(),
//!            msg: "User not found".to_string(),
//!         }));
//!         // Or
//!         // err!("user_not_found", "User not found");
//!     }
//! }
//! ```
//!
//! ### Throw an 500 internal error
//!
//!
//! For any unhandled errors, such as network timeout, database connection error, disk full, etc...
//! They are represented as `WebError::Internal(anyhow::Error)` type, and will be converted to a
//! HTTP 500 status code with a generic error message, and the error will be logged.
//!
//! ```rust
//! use anyhow::bail;
//!
//! async fn greet_handler(uid: Query<String>) -> WebResult<String> {
//!     match do_something() {
//!         Ok(_) => ok!("Success"),
//!         Err(e) => Err(WebError::Internal(e.into())),
//!     }
//! }
//!
//! fn do_something() -> anyhow::Result<()> {
//!     bail!("You can't do this");
//! }
//! ```
//!
//! Since we implemented trait `From<Into<anyhow::Error>>` for `WebError<E>`, we can use the `?` operator in the handler to directly throw an `Into<anyhow::Error>`, and get the 500 http response.
//!
//! ```rust
//! async fn greet_handler() -> WebResult<String> {
//!    do_something()?; // This will return a `WebError::Internal(anyhow::Error)` if `do_something()` returns an error
//!    ok!("Success")
//! }
//! ```
//!

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::any::Any;

/// Build a `WebError::Business<CommonError>` error, just like `anyhow!()`
#[macro_export]
macro_rules! common {
    ($code:expr, $($arg:tt)*) => {
        crate::web::result::WebError::common($code, &format!($($arg)*))
    };
}

/// Build and return a `WebError::Business<CommonError>` error, just like `bail!()`
#[macro_export]
macro_rules! err {
    ($code:expr, $($arg:tt)*) => {
        return Err($crate::web::result::WebError::common($code, &format!($($arg)*)))
    };
}

/// Build and return a `WebResponse::ok` result as `axum::Json`
#[macro_export]
macro_rules! ok {
    ($data:expr) => {
        return Ok(axum::Json($crate::web::result::WebResponse::ok($data)))
    };
}

/// This is used as the return type for web handler functions.
///
/// # Response structures
///
/// We have different strategies for success responses and error responses
///
/// ## Success responses
///
/// For any success response, we use [WebResponse] as the unified structure, with `ContentType: application/json` HTTP 200 status.
///
/// ```json
/// {
///     "ok": true,
///     "data": {
///         // The success data
///     }
/// }
/// ```
///
/// The success response in code is represented as `Result::Ok(Json<WebResponse<T>>)`
///
/// ## Error responses
///
/// The error is represented with enum [WebError], there are to type of errors.
///
/// ### Business errors
///
/// Known errors for business logics, not related to application code errors.
///
/// We use `WebResponse` with a serializable error data type. JSON and HTTP 200 status.
///
/// The error data type is by default to [CommonError].
///
/// For example:
///
/// ```json
/// {
///     "ok": false,
///     "data": {
///         "code": "permission_denied",
///         "msg": "You don't have permission to access this resource"
///     }
/// }
/// ```
///
/// The business error is represented as `Result::Err<WebError::Business<CommonError>>` type, then handled by the `impl IntoResponse for WebError<E>`.
///
/// ### Internal errors
///
/// Unhandled errors, usually throw by the `?` operator.
///
/// For example: network timeout, database connection error, disk full, etc...
///
/// Once it is returned, we respond the HTTP 500 status.
///
/// ```http
/// HTTP/1.1 500 Internal Server Error
///
/// Something went wrong
/// ```
///
/// It's represented as `Result::Error<WebError::Internal<anyhow::Error>>` type, then handled by the `impl IntoResponse for WebError<E>`.
///
/// ### Other status code
///
/// We also use other HTTP status code for, but they are not related to a handler
///
/// - `4XX`: Bad Request, Not Authorized, Forbidden, Not Found, etc...
///
/// ## Macros
///
/// And there are two convenience macros:
/// - `ok!(data)` Returns the data as success
/// - `err!(code, msg)` Returns a business
///
/// ## The `?` operator
///
/// Since the `From<Into<anyhow::Error>>` is implemented for the [WebError] You can use `?` in the handler to directly throw an `Into<anyhow::Error>`, and get the 500 http response.
///
pub type WebResult<T, E = CommonError> = Result<Json<WebResponse<T>>, WebError<E>>;

/// The unified JSON response structure for almost all http handlers in this project.
///
/// Example:
///
/// ```json
/// {
///     "ok": true,
///     "data": {
///         // Some data
///     }
/// }
/// ```
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WebResponse<T> {
    pub ok: bool,
    pub data: T,
}

/// A help type to distinguish business errors and internal errors.
/// It's useful to enable the `?` operator and the response logic.
#[derive(Debug)]
pub enum WebError<E: Serialize> {
    Business(E),
    Internal(anyhow::Error),
}

/// This is used to enable the `?` operator for every Error type that could be converted to `anyhow::Error` in handlers
impl<E, F> From<E> for WebError<F>
where
    E: Into<anyhow::Error>,
    F: Serialize,
{
    fn from(err: E) -> Self {
        Self::Internal(err.into())
    }
}


/// A common error data type used in a business error, contains a string `code` type and `msg`.
///
/// It's always used as the default error data type in responses, e.g:
///
/// ```json
/// {
///     "ok": false,
///     "data": {
///         "code": "permission_denied",
///         "msg": "You don't have permission to access this resource"
///     }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommonError {
    /// Error code type, should be machine-friendly and human-readable.
    /// e.g: `"permission_denied"`, `"not_found"`
    pub code: String,
    /// Explain the reason, could be displayed to user.
    /// e.g: `"You don't have permission to access this resource"`
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

/// This is used for handling the `WebError` return type in axum handlers, and convert it to a proper HTTP response.
/// Then we can use `Result<T, WebError<E>>` as the return type for axum handlers.
///
/// - The `WebError::Business` variant will be converted to a `WebResponse::err` JSON response with HTTP 200 status code.
/// - The `WebError::Internal` variant will be converted to a HTTP 500 status code with a generic error message, and the error will be logged.
///
impl<E: Serialize + 'static> IntoResponse for WebError<E> {
    fn into_response(self) -> axum::response::Response {
        match self {
            WebError::Business(ref resp) => {
                let value_any = resp as &dyn Any;
                if let Some(as_common_err) = value_any.downcast_ref::<CommonError>() {
                    tracing::info!(
                        error_type = "common_error",
                        error_code = as_common_err.code,
                        error_msg = as_common_err.msg,
                        "Common error"
                    )
                } else {
                    tracing::info!("Business error")
                }

                Json(WebResponse::err(resp)).into_response()
            }
            WebError::Internal(err) => {
                tracing::error!("Internal error occurs in handlers. \n{:?}", err);
                // let error_message = format!("Something went wrong: {}", err.to_string());
                // We choose not to expose the internal error message to the client, for security reasons.
                let error_message = "Something went wrong";
                (StatusCode::INTERNAL_SERVER_ERROR, error_message).into_response()
            }
        }
    }
}