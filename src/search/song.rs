use std::collections::HashMap;
use itertools::Itertools;
use meilisearch_sdk::client::{Client, SwapIndexes};
use meilisearch_sdk::errors::{Error, ErrorCode};
use meilisearch_sdk::indexes::Index;
use metrics::counter;
use crate::db::song::{ISongDao, Song, SongDao, SongOriginInfo, SongProductionCrew};
use serde::{Deserialize, Serialize};
use sqlx::{query, PgPool};
use tracing::{error, info, info_span, warn, Instrument};
use crate::db::CrudDao;
use crate::db::song_tag::{SongTag};

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
    pub origin_artists: Vec<String>,
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
    let document = convert_to_document(song_id, song_info, crew, origin_info, tags);

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

pub async fn setup_search_index(client: &Client, pg_pool: &PgPool) -> Result<(), meilisearch_sdk::errors::Error> {
    let exists = match client.get_index("songs").await {
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
        info!("Setting up songs index");
        setup_search_index_with_name(client, "songs").await?;

        // Startup indexing
        tokio::spawn({
            let client = client.clone();
            let pool = pg_pool.clone();
            async move {
                match fully_index_songs(&client, &pool).await {
                    Ok(_) => {}
                    Err(err) => {
                        error!("Failed to index songs: {:?}", err);
                    }
                };
            }.instrument(info_span!("full_index_songs"))
        });
    }

    Ok(())
}

async fn setup_search_index_with_name(client: &Client, index_name: &str) -> Result<Index, meilisearch_sdk::errors::Error> {
    let index = client.index(index_name);

    // Set searchable attributes
    index.set_searchable_attributes([
        "title",
        "subtitle",
        "artist",
        "origins",
        "origin_artists",
        "tags",
        "crew",
    ]).await?;

    // Set filterable attributes
    index.set_filterable_attributes([
        "tags",
        "creation_type",
        "uploader_uid",
        "release_time"
    ]).await?;

    // Set sortable attributes
    index.set_sortable_attributes([
        "play_count",
        "like_count",
        "release_time"
    ]).await?;

    Ok(index)
}

// Schedule to execute fully indexing task
async fn fully_index_songs(
    client: &Client,
    pool: &PgPool,
) -> anyhow::Result<()> {
    counter!("full_index_song_count").increment(1);

    // 1. Take all songs from the database (it's best to get a snapshot)
    // 2. Catch-up new changes
    // 3. Indexing
    // 4. Replace it with the new index
    let time = chrono::Utc::now();
    let new_index_name = format!("songs_{}", time.format("%Y%m%d%H%M%S"));

    // How much RAM is it required to do this job?
    let songs = SongDao::list(pool).await?;

    let new_index = setup_search_index_with_name(client, &new_index_name).await?;

    let chunks = songs.chunks(1024).collect::<Vec<_>>();

    for (index, chunk) in chunks.iter().enumerate() {
        info!("indexing chunk {} of {}", index, chunks.len());

        let mut documents: Vec<SongDocument> = vec![];
        let ids: Vec<_> = chunk.iter().map(|x| x.id).collect();
        let mut songs: HashMap<i64, _> = SongDao::list_by_ids(pool, &ids).await?.into_iter()
            .map(|x| (x.id, x))
            .collect();
        let mut crews: HashMap<i64, _> = query!(
            "SELECT song_id, u.username internal_username, c.uid, c.person_name external_username, c.role FROM song_production_crew c
               LEFT JOIN users u ON u.id = c.uid
               WHERE song_id = ANY($1)", &ids)
            .fetch_all(pool).await?
            .into_iter().into_group_map_by(|x| x.song_id);

        let mut origin_infos: HashMap<i64, _> = query!(
            "SELECT song_id,
                s.title  internal_title,
                s.artist internal_artist,
                o.origin_type,
                o.origin_song_id,
                o.origin_title,
                o.origin_artist,
                o.origin_url
            FROM song_origin_info o
                 LEFT JOIN songs s ON s.id = o.origin_song_id
            WHERE o.song_id = ANY($1)", &ids)
            .fetch_all(pool).await?
            .into_iter().into_group_map_by(|x| x.song_id);

        let mut tags: HashMap<i64, _> = query!(
            "SELECT song_id, t.name
            FROM song_tag_refs r
                LEFT JOIN song_tags t ON t.id = r.tag_id
            WHERE r.song_id = ANY($1)", &ids)
            .fetch_all(pool).await?
            .into_iter().into_group_map_by(|x| x.song_id);


        for id in ids {
            let song_info = if let Some(x) = songs.get(&id) {
                x
            } else {
                warn!("Song not found for id: {}", id);
                continue;
            };

            let origin_titles = origin_infos.get_mut(&id).unwrap_or(&mut vec![])
                .into_iter()
                .map(|x| x.internal_title.take().or(x.origin_title.take()).unwrap_or("Unknown".to_string()))
                .collect();

            let origin_artists = origin_infos.get_mut(&id).unwrap_or(&mut vec![])
                .into_iter()
                .map(|x| x.internal_artist.take().or(x.origin_artist.take()).unwrap_or("Unknown".to_string()))
                .collect();

            let crew_names = crews.get_mut(&id).unwrap_or(&mut vec![])
                .into_iter()
                .map(|x| x.internal_username.take()
                    .or(x.external_username.take())
                    .unwrap_or("Unknown".to_string())
                    .to_string()
                ).collect::<Vec<_>>();

            let tag_names = tags.get_mut(&id).unwrap_or(&mut vec![])
                .into_iter().map(|x| x.name.take().unwrap_or("Unknown".to_string()))
                .collect();

            let doc = SongDocument {
                id: id,
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
                origin_artists: origin_artists,
                crew: crew_names,
                release_time: song_info.release_time.timestamp(),
            };
            documents.push(doc)
        }

        info!("sync chunk {} to MeiliSearch: {:?}", index, documents.len());
        let _ = new_index.add_documents(&documents, Some("id")).await?
            .wait_for_completion(&client, None, None)
            .await?;
        info!("sync chunk {index} successfully");
    }

    info!("sync all chunk successfully, swapping indexes");
    let _ = client.swap_indexes([&SwapIndexes {
        indexes: ("songs".to_string(), new_index_name)
    }]).await?
        .wait_for_completion(&client, None, None)
        .await?;
    info!("swapping indexes successfully");
    new_index.delete().await?;
    counter!("full_index_song_success_count").increment(1);

    Ok(())
}

pub fn convert_to_document(
    song_id: i64,
    song_info: &Song,
    crew: &[SongProductionCrew],
    origin_info: &[SongOriginInfo],
    tags: &[SongTag],
) -> SongDocument {
    let crew_names: Vec<String> = crew.iter()
        .map(|c| format!("{}: {}", c.role, c.person_name.as_deref().unwrap_or("Unknown")))
        .collect();

    let origin_titles: Vec<_> = origin_info.iter().filter_map(|x| x.origin_title.clone())
        .collect();

    let origin_artists = origin_info.iter().filter_map(|x| x.origin_artist.clone())
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
        origin_artists: origin_artists,
        crew: crew_names,
        release_time: song_info.release_time.timestamp(),
    };

    document
}