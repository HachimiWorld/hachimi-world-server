//! Gives playlists without a custom cover the cover of their most recently added song that isn't
//! hidden, as adding or removing a song does since playlists got default covers. Run it once every
//! server runs the new version.
//!
//! The `add_playlist_custom_cover` migration marks the covers set before it as custom, but servers
//! still on the old version keep saving uploads without marking them. So first, covers uploaded to
//! `images/playlist/` (see `/playlist/set_cover`) that aren't marked are marked as custom; song
//! covers are never stored there.
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

    let unmarked = sqlx::query_scalar!(
        r#"SELECT p.id as "id: PlaylistId" FROM playlists p
        WHERE NOT p.custom_cover AND p.cover_url LIKE '%/images/playlist/%'"#
    ).fetch_all(&sql_pool).await?;
    if apply {
        sqlx::query!(
            "UPDATE playlists SET custom_cover = TRUE WHERE id = ANY($1)",
            &unmarked as &[PlaylistId]
        ).execute(&sql_pool).await?;
    }

    // The same pick as `PlaylistDao::refresh_default_covers`, to count before changing anything.
    // In a dry run, the unmarked uploads are left out as if they were marked.
    let rows = sqlx::query!(
        r#"SELECT p.id as "id: PlaylistId", p.cover_url, (
            SELECT s.cover_art_url FROM playlist_songs ps JOIN songs s ON s.id = ps.song_id
            WHERE ps.playlist_id = p.id AND NOT s.is_hidden
            ORDER BY ps.add_time DESC, ps.order_index DESC
            LIMIT 1
        ) as default_cover
        FROM playlists p WHERE NOT p.custom_cover"#
    ).fetch_all(&sql_pool).await?;

    let rows: Vec<_> = rows.into_iter().filter(|x| !unmarked.contains(&x.id)).collect();
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

    let (mark_verb, verb) = if apply { ("Marked", "Updated") } else { ("Would mark", "Would update") };
    println!("{mark_verb} {} uploaded covers as custom.", unmarked.len());
    println!("{total} playlists without a custom cover. {verb} {}.", changed.len());
    Ok(())
}
