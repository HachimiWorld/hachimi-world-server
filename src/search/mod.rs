use meilisearch_sdk::client::Client;
use crate::db::song::{Song, SongOriginInfo, SongProductionCrew};
use serde::{Deserialize, Serialize};
use crate::db::song_tag::SongTag;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongDocument {
    pub id: i64,
    pub display_id: String,
    pub title: String,
    pub subtitle: String,
    pub description: String,
    pub cover_url: String,
    pub artist: String,
    // pub lyrics: String,             -- No, lyrics should not be store because the hachimi lyrics are too much similar 
    pub duration_seconds: i32,
    pub uploader_uid: i64,
    pub creation_type: i32,
    pub play_count: i64,
    pub like_count: i64,
    pub tags: Vec<String>,
    pub origins: Vec<String>,
    pub crew: Vec<String>,
    pub release_time: i64,
}

pub async fn add_song_document(
    client: &Client,
    song_id: i64,
    song_info: &Song,
    crew: &[SongProductionCrew],
    origin_info: &[SongOriginInfo],
    tags: &[SongTag],
) -> Result<(), meilisearch_sdk::errors::Error> {
    let crew_names: Vec<String> = crew.iter()
        .map(|c| format!("{}: {}", c.role, c.person_name.as_deref().unwrap_or("Unknown")))
        .collect();

    let origin_titles: Vec<_> = origin_info.iter().filter_map(|x| x.origin_title.clone())
        .collect();

    // FIXME(search): If we update the tag name, we should find a way to update the corresponding document in MeiliSearch

    let tag_names: Vec<String> = tags.iter().map(|x| x.name.clone()).collect();

    let document = SongDocument {
        id: song_id,
        display_id: song_info.display_id.clone(),
        title: song_info.title.clone(),
        subtitle: song_info.subtitle.clone(),
        description: song_info.description.clone(),
        cover_url: song_info.cover_art_url.clone(),
        artist: song_info.artist.clone(),
        // lyrics: song_info.lyrics.clone(),
        duration_seconds: song_info.duration_seconds,
        uploader_uid: song_info.uploader_uid,
        creation_type: song_info.creation_type,
        play_count: song_info.play_count,
        like_count: song_info.like_count,
        tags: tag_names,
        origins: origin_titles, // Will be populated from origin_info if needed
        crew: crew_names,
        release_time: song_info.release_time.timestamp(),
    };

    client.index("songs")
        .add_documents(&[document], Some("id"))
        .await?;

    Ok(())
}

pub async fn delete_song_document(
    client: &Client,
    song_ids: &[i64],
) -> Result<(), meilisearch_sdk::errors::Error> {
    client.index("songs")
        .delete_documents(song_ids)
        .await?;
    Ok(())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub filter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub hits: Vec<SongDocument>,
    pub query: String,
    pub processing_time_ms: u64,
    pub hits_info: SearchResultHitsInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResultHitsInfo {
    pub total_hits: Option<usize>,
    pub limit: usize,
    pub offset: usize,
}

pub async fn search_songs(
    client: &Client,
    query: &SearchQuery,
) -> Result<SearchResult, meilisearch_sdk::errors::Error> {
    let index = client.index("songs");
    let mut search_request = index.search();
    search_request
        .with_query(&query.q)
        .with_limit(query.limit.unwrap_or(20))
        .with_offset(query.offset.unwrap_or(0));

    if let Some(ref filter) = query.filter {
        search_request.with_filter(filter);
    }

    let search_results = search_request.execute::<SongDocument>().await?;

    Ok(SearchResult {
        hits: search_results.hits.into_iter().map(|x| x.result).collect(),
        query: query.q.clone(),
        processing_time_ms: search_results.processing_time_ms as u64,
        hits_info: SearchResultHitsInfo {
            total_hits: search_results.total_hits,
            limit: search_results.limit.unwrap_or(20),
            offset: search_results.offset.unwrap_or(0),
        },
    })
}

pub async fn setup_search_index(client: &Client) -> Result<(), meilisearch_sdk::errors::Error> {
    let index = client.index("songs");

    // Set searchable attributes
    index.set_searchable_attributes([
        "title",
        "subtitle",
        "description",
        "artist",
        "tags",
        "crew",
    ]).await?;

    // Set filterable attributes
    index.set_filterable_attributes([
        "creation_type",
        "uploader_uid",
        "release_time",
        "duration_seconds",
    ]).await?;

    // Set sortable attributes
    index.set_sortable_attributes([
        "play_count",
        "like_count",
        "release_time",
        "duration_seconds",
    ]).await?;

    Ok(())
}