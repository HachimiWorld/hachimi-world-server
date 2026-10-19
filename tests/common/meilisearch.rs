//! Meilisearch for integration tests.
//!
//! With `TEST_MEILISEARCH_URL` set (e.g. `http://localhost:7700`, master key `12345678`), tests share
//! that server and keep apart by giving their indexes a random prefix. Without it, each test starts
//! its own container.

use crate::common::images::Meilisearch1_32;
use hachimi_world_server::search::SearchClient;
use meilisearch_sdk::client::Client;
use meilisearch_sdk::indexes::IndexesQuery;
use std::time::{Duration, Instant};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tracing::info;

const MASTER_KEY: &str = "12345678";

/// Keeps the test indexes alive; call [TestMeilisearch::cleanup] when the test is done.
pub enum TestMeilisearch {
    Container(ContainerAsync<Meilisearch1_32>),
    Shared,
}

impl TestMeilisearch {
    pub async fn start() -> (TestMeilisearch, SearchClient) {
        match std::env::var("TEST_MEILISEARCH_URL") {
            Ok(url) => start_shared(url).await,
            Err(_) => start_container().await,
        }
    }

    /// Index settings are applied in the background, and the shared server may have a queue of
    /// other tests' tasks. Wait until the indexes exist so the first search doesn't miss them.
    pub async fn wait_for_indexes(client: &SearchClient) {
        let deadline = Instant::now() + Duration::from_secs(30);
        for name in ["songs", "users", "playlists"] {
            while client.client().get_index(client.index_name(name)).await.is_err() {
                assert!(Instant::now() < deadline, "Index {name} was not created in time");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    pub async fn cleanup(self, client: &SearchClient) {
        match self {
            TestMeilisearch::Container(instance) => drop(instance),
            TestMeilisearch::Shared => {
                // Also catches the temporary indexes of a full indexing the test ended in the middle of.
                let prefix = client.index_name("");
                let mut query = IndexesQuery::new(client.client());
                query.with_limit(10_000);
                let indexes = client.client().list_all_indexes_with(&query).await.unwrap();
                for index in indexes.results.into_iter().filter(|x| x.uid.starts_with(&prefix)) {
                    let _ = index.delete().await;
                }
            }
        }
    }
}

async fn start_container() -> (TestMeilisearch, SearchClient) {
    let instance = Meilisearch1_32::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(7700).await.unwrap();
    let url = format!("http://{host_ip}:{host_port}");
    info!("Open test meilisearch instance at {}", url);
    let client = Client::new(url, Some(MASTER_KEY)).unwrap();
    (TestMeilisearch::Container(instance), SearchClient::new(client, ""))
}

async fn start_shared(url: String) -> (TestMeilisearch, SearchClient) {
    let client = Client::new(url, Some(MASTER_KEY)).unwrap();

    // The shared server may still be starting when the first tests run.
    let deadline = Instant::now() + Duration::from_secs(60);
    while let Err(e) = client.health().await {
        assert!(Instant::now() < deadline, "Cannot reach TEST_MEILISEARCH_URL: {e}");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let prefix = format!("t{}_", uuid::Uuid::new_v4().simple());
    info!("Use index prefix {} on the shared meilisearch", prefix);
    (TestMeilisearch::Shared, SearchClient::new(client, prefix))
}
