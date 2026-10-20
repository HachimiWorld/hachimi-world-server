//! Gives playlists without a custom cover the cover of their most recently added song that isn't
//! hidden, as adding or removing a song does since playlists got default covers. Run it after the
//! `mark_custom_playlist_covers` migration, which marks the covers set before as custom.
//!
//! Dry run by default: counts what would change. Pass `--apply` to update the playlists.
//! Reads `db` from the config at `$CONFIG_PATH` (default `config.yaml`).

use hachimi_world_server::config::Config;
use hachimi_world_server::db::playlist::{PlaylistDao, PlaylistId};
use serde::Deserialize;
use std::env;

#[derive(Deserialize, Clone, Debug)]
struct DatabaseConfig {
    pub address: String,
    pub username: String,
    pub password: String,
    pub database: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    let apply = env::args().any(|x| x == "--apply");
    let config = Config::parse(env::var("CONFIG_PATH").unwrap_or_else(|_| String::from("config.yaml")))?;
    let db: DatabaseConfig = config.get_and_parse("db")?;
    let sql_pool = sqlx::PgPool::connect(&format!(
        "postgres://{}:{}@{}/{}",
        db.username, urlencoding::encode(&db.password), db.address, db.database
    )).await?;

    if !apply {
        println!("Dry run. Pass --apply to update the playlists.");
    }

    // The same pick as `PlaylistDao::refresh_default_covers`, to count before changing anything
    let rows = sqlx::query!(
        r#"SELECT p.id as "id: PlaylistId", p.cover_url, (
            SELECT s.cover_art_url FROM playlist_songs ps JOIN songs s ON s.id = ps.song_id
            WHERE ps.playlist_id = p.id AND NOT s.is_hidden
            ORDER BY ps.add_time DESC, ps.order_index DESC
            LIMIT 1
        ) as default_cover
        FROM playlists p WHERE NOT p.custom_cover"#
    ).fetch_all(&sql_pool).await?;

    let total = rows.len();
    let changed: Vec<PlaylistId> = rows.into_iter()
        .filter(|x| x.cover_url != x.default_cover)
        .map(|x| x.id)
        .collect();

    if apply {
        for chunk in changed.chunks(500) {
            PlaylistDao::refresh_default_covers(&sql_pool, chunk).await?;
        }
    }

    let verb = if apply { "Updated" } else { "Would update" };
    println!("{total} playlists without a custom cover. {verb} {}.", changed.len());
    Ok(())
}
