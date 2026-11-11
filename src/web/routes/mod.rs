//! Routes, nested by what they're about.
//!
//! Lists, for new endpoints (older ones keep their names and parameters until they need a v2):
//! - Name them by what they return, `list` or `list_<what>`, not by how they page.
//! - Page by cursor where items keep arriving or the reader scrolls on, such as feeds: query
//!   `cursor` (omit for the first page) and `limit`; response `items` and `next_cursor`, an opaque string that
//!   is `null` when there's no more. Errors with `invalid_cursor`.
//! - Page by number only where the reader jumps to a page or needs the total, such as admin lists
//!   and search: query `page_index` (from 0) and `page_size`; response `items` and `total`.

pub mod user;
pub mod auth;
pub mod email_change;
pub mod song;
pub mod playlist;
pub mod version;
pub mod play_history;
pub mod publish;
pub mod post;
pub mod contributor;
pub mod sitemap;
pub mod notification;
pub mod message;
pub mod report;
pub mod committee;
pub mod comment;
pub mod moment;
pub mod feed;

use crate::web::state::AppState;
use aide::axum::ApiRouter;
use aide::openapi::{Info, OpenApi, Server};
use aide::transform::TransformOperation;
use axum::Router;

pub fn router() -> Router<AppState> {
    api_router().into()
}

/// Notes when an operation was added, as `@since YYMMDD` in its description in the OpenAPI
/// document, the way fields note it in their doc comments:
/// `.api_route("/square", get_with(square, since("261106")))`.
pub fn since(version: &'static str) -> impl FnOnce(TransformOperation) -> TransformOperation {
    move |op| op.description(&format!("@since {version}"))
}

/// [since] for an operation that is still experimental.
pub fn since_experimental(version: &'static str) -> impl FnOnce(TransformOperation) -> TransformOperation {
    move |op| op.description(&format!("@since {version} @experimental"))
}

/// Marks an operation deprecated in the OpenAPI document, so it can be removed later without the
/// compatibility check failing: `.api_route("/recent", get_with(recent, deprecated))`. Say in the
/// handler's doc when it can go, such as
/// `@deprecated 261110: clients stopped calling it in 261107. Remove once min_version >= 261107.`
pub fn deprecated(op: TransformOperation) -> TransformOperation {
    op.with(|mut op| {
        op.inner_mut().deprecated = true;
        op
    })
}

/// The API as an OpenAPI document, for checking changes stay compatible with released clients.
/// Routes added with `.api_route` are in it; a plain `.route` on an [ApiRouter] is left out.
pub fn openapi() -> OpenApi {
    let mut api = OpenApi {
        info: Info {
            title: "Hachimi World API".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        },
        // Paths are written without the prefix the server mounts them under
        servers: vec![Server { url: "/api".to_string(), ..Default::default() }],
        ..Default::default()
    };
    // A handler whose types can't be described would leave a hole in the document
    aide::generate::on_error(|error| panic!("OpenAPI generation failed: {error}"));
    let _ = api_router().finish_api(&mut api);
    api
}

/// Tags a module's operations with its path, which documentation viewers group them by.
fn module(name: &str, router: ApiRouter<AppState>) -> ApiRouter<AppState> {
    router.with_path_items(|item| item.tag(name))
}

fn api_router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .nest("/auth", module("auth", auth::router()))
        .nest("/user", module("user", user::router()))
        .nest("/song", module("song", song::router()))
        .nest("/play_history", module("play_history", play_history::router()))
        .nest("/playlist", module("playlist", playlist::router()))
        .nest("/version", module("version", version::router()))
        .nest("/publish", module("publish", publish::router()))
        .nest("/post", module("post", post::router()))
        .nest("/contributor", module("contributor", contributor::router()))
        .nest("/sitemap", module("sitemap", sitemap::router()))
        .nest("/notification", module("notification", notification::router()))
        .nest("/message", module("message", message::router()))
        .nest("/report", module("report", report::router()))
        .nest("/committee", module("committee", committee::router()))
        .nest("/comment", module("comment", comment::router()))
        .nest("/moment", module("moment", moment::router()))
        .nest("/feed", module("feed", feed::router()))
}
#[cfg(test)]
mod tests {
    use super::deprecated;
    use crate::web::state::AppState;
    use aide::axum::routing::get_with;
    use aide::axum::ApiRouter;
    use aide::openapi::OpenApi;

    #[test]
    fn deprecated_marks_the_operation() {
        async fn old() {}
        let mut api = OpenApi::default();
        let _ = ApiRouter::<AppState>::new()
            .api_route("/old", get_with(old, deprecated))
            .finish_api(&mut api);
        let op = api.paths.unwrap().paths["/old"].as_item().unwrap().get.clone().unwrap();
        assert!(op.deprecated);
    }

    /// The committed openapi.json is what CI compares against master, so it must match the routes.
    #[test]
    fn openapi_json_is_current() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json");
        let committed = std::fs::read_to_string(path).unwrap_or_default();
        let generated = serde_json::to_string_pretty(&super::openapi()).unwrap() + "\n";
        assert!(committed == generated, "openapi.json is out of date, run `cargo run --bin gen_openapi` and commit it");
    }
}
