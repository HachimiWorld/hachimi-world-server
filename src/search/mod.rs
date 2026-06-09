use anyhow::Context;
use sqlx::PgPool;
use tokio::join;
use tracing::info;

pub mod song;
pub mod user;
pub mod playlist;

pub async fn setup_meilisearch_indexes(client: &meilisearch_sdk::client::Client, pool: &PgPool) -> anyhow::Result<()> {
    info!("Setting up search index");
    let (a, b, c) = join!(
        song::setup_search_index(&client, pool),
        user::setup_search_index(&client, pool),
        playlist::setup_search_index(&client, pool)
    );
    a.or(b).or(c).context("Failed to setup search index")
}