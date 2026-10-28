//! Counts again the derived counts on comments, `like_count` and `visible_reply_count`, and lists
//! the comments where they differ from what they count. The server keeps them in step as it
//! changes what they count, so this finds nothing unless a bug or a change made by hand let one
//! drift.
//!
//! Dry run by default: lists what differs. Pass `--apply` to set those counts to what they count.
//! Reads `db` from the config at `$CONFIG_PATH` (default `config.yaml`).

use hachimi_world_server::config::Config;
use hachimi_world_server::db::comment::{CommentDao, CommentId};
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
        println!("Dry run. Pass --apply to fix the counts.");
    }

    let miscounted = CommentDao::list_miscounted(&sql_pool).await?;
    for x in &miscounted {
        println!(
            "comment {}: like_count {} -> {}, visible_reply_count {} -> {}",
            x.id.0, x.like_count, x.likes, x.visible_reply_count, x.visible_replies
        );
    }

    if apply {
        let ids: Vec<CommentId> = miscounted.iter().map(|x| x.id).collect();
        for chunk in ids.chunks(500) {
            CommentDao::recount(&sql_pool, chunk).await?;
        }
    }

    let verb = if apply { "Fixed" } else { "Would fix" };
    println!("{verb} {} comments.", miscounted.len());
    Ok(())
}
