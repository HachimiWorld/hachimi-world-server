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
use aide::openapi::{Info, OpenApi};
use axum::Router;

pub fn router() -> Router<AppState> {
    api_router().into()
}

/// The API as an OpenAPI document, for checking changes stay compatible with released clients.
/// Only modules on [ApiRouter] routes are in it; the others are still plain axum routes.
pub fn openapi() -> OpenApi {
    let mut api = OpenApi {
        info: Info {
            title: "Hachimi World API".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    // A handler whose types can't be described would leave a hole in the document
    aide::generate::on_error(|error| panic!("OpenAPI generation failed: {error}"));
    let _ = ApiRouter::<AppState>::new()
        .nest("/api", api_router())
        .finish_api(&mut api);
    api
}

fn api_router() -> ApiRouter<AppState> {
    ApiRouter::new()
        .nest("/auth", auth::router().into())
        .nest("/user", user::router())
        .nest("/song", song::router())
        .nest("/play_history", play_history::router().into())
        .nest("/playlist", playlist::router().into())
        .nest("/version", version::router().into())
        .nest("/publish", publish::router().into())
        .nest("/post", post::router().into())
        .nest("/contributor", contributor::router().into())
        .nest("/sitemap", sitemap::router().into())
        .nest("/notification", notification::router().into())
        .nest("/message", message::router().into())
        .nest("/report", report::router().into())
        .nest("/committee", committee::router().into())
        .nest("/comment", comment::router().into())
        .nest("/moment", moment::router().into())
        .nest("/feed", feed::router().into())
}