//! Writes the sitemap files to a local directory for checking by hand, without uploading.
//! The server generates and uploads them on its own schedule (see `service::sitemap`).
//!
//! Usage: `cargo run --bin gen_sitemap [out_dir]` (default `.local/sitemap-out`).
//! Reads `db` from the config at `$CONFIG_PATH` (default `config.yaml`), and `sitemap` if present.

use hachimi_world_server::config::Config;
use hachimi_world_server::db::song::SongDao;
use hachimi_world_server::service::sitemap::{self, SitemapCfg};
use serde::Deserialize;
use std::path::Path;
use std::{env, fs};

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
    let config = Config::parse(env::var("CONFIG_PATH").unwrap_or_else(|_| String::from("config.yaml")))?;
    let out_dir = env::args().nth(1).unwrap_or_else(|| String::from(".local/sitemap-out"));

    let db: DatabaseConfig = config.get_and_parse("db")?;
    let sql_pool = sqlx::PgPool::connect(&format!(
        "postgres://{}:{}@{}/{}",
        db.username, urlencoding::encode(&db.password), db.address, db.database
    )).await?;

    let cfg = match config.get("sitemap")? {
        Some(_) => config.get_and_parse::<SitemapCfg>("sitemap")?,
        None => SitemapCfg {
            site_url: String::from("https://hachimi.world"),
            path_secret: String::from("local-preview-secret"),
        },
    };
    cfg.validate()?;

    let entries = SongDao::list_sitemap_entries(&sql_pool).await?;
    let files = sitemap::generate(&entries, &cfg)?;
    for file in &files {
        let path = Path::new(&out_dir).join(&file.key);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(&path, &file.content)?;
        println!("{} ({} bytes)", path.display(), file.content.len());
    }
    println!("{} songs in {} files", entries.len(), files.len());
    Ok(())
}
