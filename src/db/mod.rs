use sqlx::PgExecutor;

/// Defines a typed id, so a user id can't be passed where a song id is expected.
///
/// It is a plain `i64` underneath, stored as `BIGINT` and serialized as a number, so the
/// database schema and the JSON api stay the same.
///
/// In `query!` / `query_as!`, read a column as an id with `id as "id: _"` (or `"id: SongId"`),
/// and bind an id with `song_id as _`.
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize, sqlx::Type)]
        #[serde(transparent)]
        #[sqlx(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }

        impl redis::ToRedisArgs for $name {
            fn write_redis_args<W: ?Sized + redis::RedisWrite>(&self, out: &mut W) {
                self.0.write_redis_args(out)
            }
        }
    };
}


pub mod refresh_token;
pub mod user;
pub mod song;
pub mod song_tag;
pub mod playlist;
pub mod song_publishing_review;
pub mod song_publishing_review_comment;
pub mod song_publishing_review_history;
pub mod version;
pub mod creator;
pub mod post;
pub mod user_play_history;
pub mod user_connection_accounts;
pub mod follow;
pub mod sitemap_generation;
pub mod notification;
pub mod email_outbox;
pub mod message_read_mark;
pub mod received_like;
pub mod report;
pub mod committee_member;
pub mod content_check;

pub trait CrudDao<'e, E>
where E: PgExecutor<'e> {
    type Entity;
    type Id;
    fn list(executor: E) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>> + Send;
    fn page(executor: E, page: i64, size: i64) -> impl Future<Output = sqlx::Result<Vec<Self::Entity>>> + Send;

    fn get_by_id(executor: E, id: Self::Id) -> impl Future<Output = sqlx::Result<Option<Self::Entity>>> + Send;
    fn update_by_id(executor: E, value: &Self::Entity) -> impl Future<Output = sqlx::Result<()>> + Send;
    fn insert(executor: E, value: &Self::Entity) -> impl Future<Output = sqlx::Result<Self::Id>> + Send;
    fn delete_by_id(executor: E, id: Self::Id) -> impl Future<Output = sqlx::Result<()>> + Send;
}