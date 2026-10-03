//! Sitemap generation, daily and on demand (`/api/sitemap/generate`). Each run is recorded in
//! `sitemap_generations`.
//!
//! Song sitemaps are written to object storage under `sitemap/<path_secret>/`, and the website
//! worker serves that directory at `https://hachimi.world/sitemap/<path_secret>/`. The secret keeps
//! the sitemap from being found by anyone but the search engines it is submitted to, so it is
//! deliberately not listed in robots.txt.

use crate::db::sitemap_generation::{SitemapGeneration, SitemapGenerationDao};
use crate::db::song::{SongDao, SongSitemapEntry};
use crate::db::CrudDao;
use crate::file_hosting::FileHost;
use crate::web::state::AppState;
use bytes::Bytes;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use sitemap_rs::sitemap::Sitemap;
use sitemap_rs::sitemap_index::SitemapIndex;
use sitemap_rs::url::Url;
use sitemap_rs::url_set::UrlSet;
use tokio_cron_scheduler::{Job, JobScheduler};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

/// The sitemap protocol allows at most 50,000 URLs per file.
const URLS_PER_FILE: usize = 50_000;
const STORAGE_PREFIX: &str = "sitemap";
/// Every day at 04:00 Beijing time (sec min hour day month weekday).
const SCHEDULE: &str = "0 0 4 * * *";
const LOCK_NAME: &str = "sitemap:generate";

pub const TRIGGER_SCHEDULE: &str = "schedule";
pub const TRIGGER_MANUAL: &str = "manual";
const STATUS_RUNNING: &str = "running";
const STATUS_SUCCESS: &str = "success";
const STATUS_FAILURE: &str = "failure";

/// When every song page last changed for reasons outside the song itself (server rendering
/// on 2026-10-01, lowercase canonical URLs on 2026-10-03). Used as the floor for `lastmod`, so
/// search engines recrawl songs that haven't been edited since. Bump it when the song page
/// changes for all songs.
fn pages_changed_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 3, 5, 17, 16).unwrap()
}

/// @since 261002
#[derive(Deserialize, Clone, Debug)]
pub struct SitemapCfg {
    /// Public site origin used in `<loc>`, e.g. `https://hachimi.world`.
    /// @since 261002
    pub site_url: String,
    /// Random path segment the sitemap is published under. Letters, digits, `-` and `_` only.
    /// @since 261002
    pub path_secret: String,
    /// Bearer token for `/api/sitemap/generate` and `/api/sitemap/generation/list`.
    /// @since 261003
    pub trigger_token: String,
}

impl SitemapCfg {
    pub fn validate(&self) -> anyhow::Result<()> {
        let valid = self.path_secret.len() >= 16
            && self.path_secret.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        anyhow::ensure!(valid, "sitemap.path_secret must be at least 16 characters of [A-Za-z0-9_-]");
        anyhow::ensure!(self.trigger_token.len() >= 16, "sitemap.trigger_token must be at least 16 characters");
        Ok(())
    }
}

pub async fn start_daily_job(state: AppState, cfg: SitemapCfg, cancel_token: CancellationToken) -> anyhow::Result<()> {
    let mut scheduler = JobScheduler::new().await?;
    scheduler.add(Job::new_async_tz(SCHEDULE, chrono_tz::Asia::Shanghai, move |_, _| {
        let state = state.clone();
        let cfg = cfg.clone();
        Box::pin(async move {
            if let Err(e) = run(&state, &cfg, TRIGGER_SCHEDULE).await {
                error!("Failed to generate sitemap: {e:?}");
            }
        })
    })?).await?;
    scheduler.start().await?;

    tokio::spawn(async move {
        cancel_token.cancelled().await;
        if let Err(e) = scheduler.shutdown().await {
            error!("Failed to shut down sitemap scheduler: {e:?}");
        }
    });
    Ok(())
}

/// Generates and uploads the sitemaps, recording the run. Returns `None` without doing anything
/// if another run (on any replica) holds the lock. A failed generation is recorded and returned
/// as a `failure` record; only failing to record it is an `Err`.
pub async fn run(state: &AppState, cfg: &SitemapCfg, trigger_type: &str) -> anyhow::Result<Option<SitemapGeneration>> {
    let Some(_guard) = state.red_lock.try_lock(LOCK_NAME).await? else {
        info!("Sitemap is being generated elsewhere, skipping");
        return Ok(None);
    };

    let mut generation = SitemapGeneration {
        id: 0,
        trigger_type: trigger_type.to_string(),
        status: STATUS_RUNNING.to_string(),
        song_count: None,
        file_count: None,
        error: None,
        start_time: Utc::now(),
        finish_time: None,
    };
    generation.id = SitemapGenerationDao::insert(&state.sql_pool, &generation).await?;

    match generate_and_upload(state, cfg).await {
        Ok((song_count, file_count)) => {
            info!("Sitemap generated with {song_count} songs in {file_count} files");
            generation.status = STATUS_SUCCESS.to_string();
            generation.song_count = Some(song_count);
            generation.file_count = Some(file_count);
        }
        Err(e) => {
            error!("Failed to generate sitemap: {e:?}");
            generation.status = STATUS_FAILURE.to_string();
            generation.error = Some(format!("{e:#}"));
        }
    }
    generation.finish_time = Some(Utc::now());
    SitemapGenerationDao::update_by_id(&state.sql_pool, &generation).await?;
    Ok(Some(generation))
}

/// Returns the song and file counts.
async fn generate_and_upload(state: &AppState, cfg: &SitemapCfg) -> anyhow::Result<(i32, i32)> {
    let entries = SongDao::list_sitemap_entries(&state.sql_pool).await?;
    let files = generate(&entries, cfg)?;
    upload(state.file_host.as_ref(), &files).await?;
    Ok((entries.len().try_into()?, files.len().try_into()?))
}

/// A sitemap file to be written to object storage.
#[derive(Debug)]
pub struct SitemapFile {
    /// Object key, e.g. `sitemap/<path_secret>/songs-0.xml`.
    pub key: String,
    pub content: Bytes,
}

/// Builds the song sitemaps followed by the index that lists them. The index is always last.
pub fn generate(entries: &[SongSitemapEntry], cfg: &SitemapCfg) -> anyhow::Result<Vec<SitemapFile>> {
    generate_chunked(entries, cfg, URLS_PER_FILE)
}

fn generate_chunked(entries: &[SongSitemapEntry], cfg: &SitemapCfg, urls_per_file: usize) -> anyhow::Result<Vec<SitemapFile>> {
    let site_url = cfg.site_url.trim_end_matches('/');
    let storage_dir = format!("{STORAGE_PREFIX}/{}", cfg.path_secret);
    let public_dir = format!("{site_url}/sitemap/{}", cfg.path_secret);
    let floor = pages_changed_at();

    let mut files = Vec::new();
    let mut sitemaps = Vec::new();
    for (i, chunk) in entries.chunks(urls_per_file).enumerate() {
        let urls = chunk.iter()
            // Song URLs are lowercase; the website redirects other casings there
            .map(|x| Url::builder(format!("{site_url}/song/{}", x.display_id.to_lowercase()))
                .last_modified(x.update_time.max(floor).fixed_offset())
                .build())
            .collect::<Result<Vec<_>, _>>()?;
        let mut content = Vec::new();
        UrlSet::new(urls)?.write(&mut content)?;

        let name = format!("songs-{i}.xml");
        let last_modified = chunk.iter().map(|x| x.update_time.max(floor)).max().map(|t| t.fixed_offset());
        sitemaps.push(Sitemap::new(format!("{public_dir}/{name}"), last_modified));
        files.push(SitemapFile { key: format!("{storage_dir}/{name}"), content: content.into() });
    }

    let mut content = Vec::new();
    SitemapIndex::new(sitemaps)?.write(&mut content)?;
    files.push(SitemapFile { key: format!("{storage_dir}/index.xml"), content: content.into() });
    Ok(files)
}

/// Uploads in order, so the index (last) never points at a file that isn't there yet.
pub async fn upload(file_host: &dyn FileHost, files: &[SitemapFile]) -> anyhow::Result<()> {
    for file in files {
        file_host.upload(file.content.clone(), &file.key).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_hosting::{MockFileHost, UploadResult};
    use aws_sdk_s3::operation::put_object::PutObjectOutput;
    use chrono::{TimeZone, Utc};

    fn cfg() -> SitemapCfg {
        SitemapCfg {
            site_url: "https://hachimi.world/".to_string(),
            path_secret: "0123456789abcdef".to_string(),
            trigger_token: "0123456789abcdef".to_string(),
        }
    }

    /// A song last updated on the given day of 2026-11, after `pages_changed_at`.
    fn entry(display_id: &str, day: u32) -> SongSitemapEntry {
        SongSitemapEntry { display_id: display_id.to_string(), update_time: Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap() }
    }

    fn text(file: &SitemapFile) -> &str {
        std::str::from_utf8(&file.content).unwrap()
    }

    #[test]
    fn test_schedule_parses() {
        assert!(Job::new_async_tz(SCHEDULE, chrono_tz::Asia::Shanghai, |_, _| Box::pin(async {})).is_ok());
    }

    #[test]
    fn test_generate_splits_into_files_with_index_last() {
        let entries = [entry("JM-AAA-1", 1), entry("JM-AAA-2", 3), entry("JM-BBB-1", 2)];
        let files = generate_chunked(&entries, &cfg(), 2).unwrap();

        let keys = files.iter().map(|f| f.key.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, [
            "sitemap/0123456789abcdef/songs-0.xml",
            "sitemap/0123456789abcdef/songs-1.xml",
            "sitemap/0123456789abcdef/index.xml",
        ]);

        let songs_0 = text(&files[0]);
        assert!(songs_0.contains("<loc>https://hachimi.world/song/jm-aaa-1</loc>"));
        assert!(songs_0.contains("<loc>https://hachimi.world/song/jm-aaa-2</loc>"));
        assert!(!songs_0.contains("jm-bbb-1"));
        assert!(songs_0.contains("<lastmod>2026-11-01T00:00:00+00:00</lastmod>"));
        assert!(text(&files[1]).contains("<loc>https://hachimi.world/song/jm-bbb-1</loc>"));

        let index = text(&files[2]);
        assert!(index.contains("<loc>https://hachimi.world/sitemap/0123456789abcdef/songs-0.xml</loc>"));
        assert!(index.contains("<loc>https://hachimi.world/sitemap/0123456789abcdef/songs-1.xml</loc>"));
        // Each file's lastmod is the latest song update in it
        assert!(index.contains("<lastmod>2026-11-03T00:00:00+00:00</lastmod>"));
        assert!(index.contains("<lastmod>2026-11-02T00:00:00+00:00</lastmod>"));
    }

    #[test]
    fn test_generate_floors_lastmod_at_pages_changed_at() {
        let old = SongSitemapEntry { display_id: "JM-OLD-1".to_string(), update_time: Utc.with_ymd_and_hms(2025, 9, 1, 0, 0, 0).unwrap() };
        let files = generate(&[old], &cfg()).unwrap();
        let floor = format!("<lastmod>{}</lastmod>", pages_changed_at().format("%Y-%m-%dT%H:%M:%S+00:00"));
        assert!(text(&files[0]).contains(&floor));
        assert!(text(&files[1]).contains(&floor));
    }

    #[test]
    fn test_generate_without_songs_writes_empty_index() {
        let files = generate(&[], &cfg()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].key, "sitemap/0123456789abcdef/index.xml");
        assert!(!text(&files[0]).contains("<sitemap>"));
    }

    #[tokio::test]
    async fn test_upload_keeps_order() {
        let files = generate_chunked(&[entry("JM-AAA-1", 1), entry("JM-AAA-2", 2)], &cfg(), 1).unwrap();
        let mut file_host = MockFileHost::new();
        let mut seq = mockall::Sequence::new();
        for key in ["songs-0.xml", "songs-1.xml", "index.xml"] {
            let key = format!("sitemap/0123456789abcdef/{key}");
            file_host.expect_upload()
                .withf(move |_, k| k == key)
                .times(1)
                .in_sequence(&mut seq)
                .returning(|_, _| Box::pin(async {
                    Ok(UploadResult { output: PutObjectOutput::builder().build(), public_url: String::new() })
                }));
        }
        upload(&file_host, &files).await.unwrap();
    }
}
