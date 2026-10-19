use anyhow::Context;
use meilisearch_sdk::client::Client;
use meilisearch_sdk::indexes::Index;
use sqlx::PgPool;
use tokio::join;
use tracing::info;

pub mod song;
pub mod user;
pub mod playlist;

/// A Meilisearch client whose index names start with `prefix`, so several environments can share
/// one server. Integration tests give each test its own prefix; the server uses none.
#[derive(Clone)]
pub struct SearchClient {
    client: Client,
    prefix: String,
}

impl SearchClient {
    pub fn new(client: Client, prefix: impl Into<String>) -> Self {
        Self { client, prefix: prefix.into() }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The real name of the index called `name`, e.g. `songs`.
    pub fn index_name(&self, name: &str) -> String {
        format!("{}{}", self.prefix, name)
    }

    pub fn index(&self, name: &str) -> Index {
        self.client.index(self.index_name(name))
    }
}

/// Creates the indexes that are missing. With `fill`, also fills them from the database in the
/// background, which the server does on startup. Tests start from an empty database and skip it,
/// as a fill finishing late would swap out the documents the test has added.
pub async fn setup_meilisearch_indexes(client: &SearchClient, pool: &PgPool, fill: bool) -> anyhow::Result<()> {
    info!("Setting up search index");
    let (a, b, c) = join!(
        song::setup_search_index(&client, pool, fill),
        user::setup_search_index(&client, pool, fill),
        playlist::setup_search_index(&client, pool, fill)
    );
    a.context("Failed to setup song search index")?;
    b.context("Failed to setup user search index")?;
    c.context("Failed to setup playlist search index")?;
    Ok(())
}
