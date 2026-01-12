use meilisearch_sdk::client::{Client, SwapIndexes};
use meilisearch_sdk::errors::{Error, ErrorCode};
use meilisearch_sdk::indexes::Index;
use metrics::counter;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{error, info, info_span, Instrument};
use crate::db::CrudDao;
use crate::db::user::UserDao;
use crate::search::song::SearchResultHitsInfo;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDocument {
    pub id: i64,
    pub avatar_url: Option<String>,
    pub name: String,
    pub follower_count: i64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<UserDocument>,
    pub query: String,
    pub processing_time_ms: u64,
    pub hits_info: SearchResultHitsInfo,
}

/// Search users in MeiliSearch
///
/// # Arguments
///
/// * `client` - MeiliSearch client instance
/// * `query` - Search query string
/// * `limit` - Maximum number of results to return
/// * `offset` - Number of results to skip
///
/// # Returns
///
/// Returns a Result containing a vector of UserDocument matches
pub async fn search_users(
    client: &Client,
    query: &str,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<SearchResult, meilisearch_sdk::errors::Error> {
    let search_results = client.index("users")
        .search()
        .with_query(query)
        .with_limit(limit.unwrap_or(20))
        .with_offset(offset.unwrap_or(0))
        .execute::<UserDocument>()
        .await?;

    let r = SearchResult {
        hits: search_results.hits.into_iter().map(|hit| hit.result).collect(),
        query: query.to_string(),
        processing_time_ms: search_results.processing_time_ms as u64,
        hits_info: SearchResultHitsInfo {
            total_hits: search_results.total_hits,
            limit: search_results.limit.unwrap_or(20),
            offset: search_results.offset.unwrap_or(0),
        },
    };
    Ok(r)
}

pub async fn update_user_document(client: &Client, document: UserDocument) -> anyhow::Result<()> {
    client.index("users")
        .add_documents(&[document], Some("id"))
        .await?;
    Ok(())
}

pub async fn setup_search_index(client: &Client, pg_pool: &PgPool) -> Result<(), meilisearch_sdk::errors::Error> {
    let exists = match client.get_index("users").await {
        Ok(_) => { true }
        Err(Error::Meilisearch(err)) => {
            if err.error_code == ErrorCode::IndexNotFound {
                false
            } else {
                Err(err)?
            }
        }
        Err(err) => Err(err)?
    };

    if !exists {
        info!("Setting up users index");
        setup_search_index_with_name(client, "users").await?;

        // Startup indexing
        tokio::spawn({
            let client = client.clone();
            let pool = pg_pool.clone();
            async move {
                match fully_index_users(&client, &pool).await {
                    Ok(_) => {}
                    Err(err) => {
                        error!("Failed to index users: {:?}", err);
                    }
                };
            }.instrument(info_span!("full_index_users"))
        });
    }

    Ok(())
}

async fn setup_search_index_with_name(client: &Client, index_name: &str) -> Result<Index, meilisearch_sdk::errors::Error> {
    let index = client.index(index_name);

    // Set searchable attributes
    index.set_searchable_attributes(["name"]).await?;
    // Set sortable attributes
    index.set_sortable_attributes(["follower_count"]).await?;
    Ok(index)
}

async fn fully_index_users(
    client: &Client,
    pool: &PgPool,
) -> anyhow::Result<()> {
    counter!("full_index_user_count").increment(1);
    let users = UserDao::list(pool).await?;
    let time = chrono::Utc::now();
    let new_index_name = format!("users_{}", time.format("%Y%m%d%H%M%S"));
    let new_index = setup_search_index_with_name(client, &new_index_name).await?;
    let chunks = users.chunks(1024).collect::<Vec<_>>();

    for (index, chunk) in chunks.iter().enumerate() {
        info!("indexing chunk {} of {}", index, chunks.len());

        let documents = chunk.iter().map(|x| UserDocument {
            id: x.id,
            name: x.username.clone(),
            avatar_url: x.avatar_url.clone(),
            follower_count: 0, // TODO: Count follower count
        }).collect::<Vec<_>>();

        info!("syncing chunk {} to MeiliSearch: {:?}", index, documents.len());
        let _ = new_index.add_documents(&documents, Some("id")).await?
            .wait_for_completion(&client, None, None)
            .await?;
        info!("sync chunk {index} successfully");
    }

    info!("sync all chunk successfully, swapping indexes");

    let _ = client.swap_indexes([&SwapIndexes {
        indexes: ("users".to_string(), new_index_name),
        rename: None,
    }]).await?
        .wait_for_completion(&client, None, None);
    info!("swapping indexes successfully");
    new_index.delete().await?;

    counter!("full_index_user_success_count").increment(1);
    Ok(())
}