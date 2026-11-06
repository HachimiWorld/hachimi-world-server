//! Routes, nested by what they're about.
//!
//! Lists, for new endpoints (older ones keep their names and parameters until they need a v2):
//! - Name them by what they return, `list` or `list_<what>`, not by how they page.
//! - Page by cursor where items keep arriving or the reader scrolls on, such as feeds: query
//!   `cursor` (omit for the first page) and `limit`; response `next_cursor`, an opaque string that
//!   is `null` when there's no more. Errors with `invalid_cursor`.
//! - Page by number only where the reader jumps to a page or needs the total, such as admin lists
//!   and search: query `page_index` (from 0) and `page_size`; response `total`.

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
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/user", user::router())
        .nest("/song", song::router())
        .nest("/play_history", play_history::router())
        .nest("/playlist", playlist::router())
        .nest("/version", version::router())
        .nest("/publish", publish::router())
        .nest("/post", post::router())
        .nest("/contributor", contributor::router())
        .nest("/sitemap", sitemap::router())
        .nest("/notification", notification::router())
        .nest("/message", message::router())
        .nest("/report", report::router())
        .nest("/committee", committee::router())
        .nest("/comment", comment::router())
        .nest("/moment", moment::router())
        .nest("/feed", feed::router())
}