use crate::db::playlist::{IPlaylistDao, PlaylistDao};
use chrono::{DateTime, Utc};
use itertools::Itertools;
use meilisearch_sdk::client::{Client, SwapIndexes};
use meilisearch_sdk::errors::{Error, ErrorCode};
use meilisearch_sdk::indexes::Index;
use metrics::counter;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{error, info, info_span, Instrument};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistDocument {
    pub id: i64,
    pub user_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<PlaylistDocument>,
    pub query: String,
    pub processing_time_ms: u64,
    pub hits_info: crate::search::song::SearchResultHitsInfo,
}

pub async fn add_or_replace_document(
    client: &Client,
    pool: &PgPool,
    playlist_ids: &[i64]
) -> anyhow::Result<()> {
    let documents = get_documents_batch(pool, playlist_ids).await?;
    client.index("playlists")
        .add_or_replace(&documents, Some("id"))
        .await?;
    Ok(())
}

pub async fn delete_playlist_document(
    client: &Client,
    playlist_ids: &[i64],
) -> Result<(), meilisearch_sdk::errors::Error> {
    client.index("playlists")
        .delete_documents(playlist_ids)
        .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub filter: Option<String>,
    pub sort_method: Option<SearchSortMethod>
}

#[derive(Debug, Clone)]
pub enum SearchSortMethod {
    CreateTimeDesc,
    CreateTimeAsc,
    UpdateTimeDesc,
    UpdateTimeAsc,
}

impl SearchSortMethod {
    fn to_meilisearch_sort(&self) -> &'static str {
        match self {
            SearchSortMethod::CreateTimeDesc => "create_time:desc",
            SearchSortMethod::CreateTimeAsc => "create_time:asc",
            SearchSortMethod::UpdateTimeDesc => "update_time:desc",
            SearchSortMethod::UpdateTimeAsc => "update_time:asc",
        }
    }
}

pub async fn search_playlists(
    client: &Client,
    query: &SearchQuery,
) -> Result<SearchResult, meilisearch_sdk::errors::Error> {
    let index = client.index("playlists");
    let mut search_request = index.search();

    let mut sort_params = vec![];
    if let Some(ref s) = query.sort_method {
        sort_params.push(s.to_meilisearch_sort());
    }
    search_request
        .with_query(&query.q)
        .with_limit(query.limit.unwrap_or(20))
        .with_offset(query.offset.unwrap_or(0));
    search_request.with_sort(&sort_params);

    if let Some(ref filter) = query.filter {
        search_request.with_filter(filter);
    }

    let search_results = search_request.execute::<PlaylistDocument>().await?;

    Ok(SearchResult {
        hits: search_results.hits.into_iter().map(|x| x.result).collect(),
        query: query.q.clone(),
        processing_time_ms: search_results.processing_time_ms as u64,
        hits_info: crate::search::song::SearchResultHitsInfo {
            total_hits: search_results.total_hits,
            limit: search_results.limit.unwrap_or(20),
            offset: search_results.offset.unwrap_or(0),
        },
    })
}

pub async fn setup_search_index(client: &Client, pg_pool: &PgPool) -> Result<(), meilisearch_sdk::errors::Error> {
    let exists = match client.get_index("playlists").await {
        Ok(_) => true,
        Err(Error::Meilisearch(err)) => {
            if err.error_code == ErrorCode::IndexNotFound {
                false
            } else {
                Err(err)?
            }
        }
        Err(err) => Err(err)?,
    };

    if !exists {
        info!("Setting up playlists index");
        setup_search_index_with_name(client, "playlists").await?;

        // Startup indexing
        tokio::spawn({
            let client = client.clone();
            let pool = pg_pool.clone();
            async move {
                match fully_index_playlists(&client, &pool).await {
                    Ok(_) => {}
                    Err(err) => {
                        error!("Failed to index playlists: {:?}", err);
                    }
                };
            }
            .instrument(info_span!("full_index_playlists"))
        });
    }

    Ok(())
}

async fn setup_search_index_with_name(client: &Client, index_name: &str) -> Result<Index, meilisearch_sdk::errors::Error> {
    let index = client.index(index_name);

    // Search text only should come from these fields.
    index.set_searchable_attributes(["title", "description"]).await?;

    // Only public playlists should be searchable.
    index.set_filterable_attributes(["user_id"]).await?;

    index.set_sortable_attributes(["create_time", "update_time"]).await?;

    Ok(index)
}

async fn fully_index_playlists(
    client: &Client,
    pool: &PgPool,
) -> anyhow::Result<()> {
    counter!("full_index_playlist_count").increment(1);

    let time = chrono::Utc::now();
    let new_index_name = format!("playlists_{}", time.format("%Y%m%d%H%M%S"));

    // Only public playlists are indexed.
    let ids = sqlx::query!("SELECT id FROM playlists WHERE is_public = true")
        .fetch_all(pool).await?
        .into_iter()
        .map(|x| x.id)
        .collect_vec();

    let new_index = setup_search_index_with_name(client, &new_index_name).await?;

    let chunks = ids.chunks(1024).collect::<Vec<_>>();
    for (index, chunk) in chunks.iter().enumerate() {
        info!("indexing chunk {} of {}", index, chunks.len());

        let documents = get_documents_batch(pool, chunk).await?;
        info!("syncing chunk {} to MeiliSearch: {:?}", index, documents.len());

        let _ = new_index
            .add_documents(&documents, Some("id"))
            .await?
            .wait_for_completion(&client, None, None)
            .await?;
        info!("sync chunk {index} successfully");
    }

    info!("sync all chunk successfully, swapping indexes");
    let _ = client
        .swap_indexes([&SwapIndexes {
            indexes: ("playlists".to_string(), new_index_name),
            rename: None,
        }])
        .await?
        .wait_for_completion(&client, None, None)
        .await?;

    info!("swapping indexes successfully");
    new_index.delete().await?;
    counter!("full_index_playlist_success_count").increment(1);

    Ok(())
}

async fn get_documents_batch(pool: &PgPool, playlist_ids: &[i64]) -> anyhow::Result<Vec<PlaylistDocument>> {
    let rows = PlaylistDao::list_by_ids(pool, playlist_ids).await?;
    let docs = rows.into_iter()
        .filter(|x| x.is_public)
        .map(|x| PlaylistDocument {
            id: x.id,
            user_id: x.user_id,
            title: x.name,
            description: x.description,
            create_time: x.create_time,
            update_time: x.update_time,
        })
        .collect_vec();
    Ok(docs)
}