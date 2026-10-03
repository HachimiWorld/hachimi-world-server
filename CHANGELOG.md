# Changelog

## Unreleased

### New

- Daily sitemap generation, enabled by the new optional `sitemap` config (`site_url`, `path_secret`, `trigger_token`). Uploads `sitemap/<path_secret>/index.xml` and `songs-<n>.xml` to s3; hachimi.world serves them under `/sitemap/<path_secret>/`.
- `/sitemap/generate` (POST)
  - Generates and uploads the sitemaps now; returns the generation record (`status` is `failure` if it failed). Errors with `generation_running` if one is in progress
  - Requires `Authorization: Bearer <sitemap.trigger_token>`
- `/sitemap/generation/list` (GET)
  - Generation records, scheduled and manual, most recent first. Query: `page_index`, `page_size` (1..=50). Response: `data`, `page_index`, `page_size`, `total`
  - Requires `Authorization: Bearer <sitemap.trigger_token>`

### Changes

- `/version/publish`
  - New optional request fields:
    - `size: Option<i64>` package size in bytes, must be positive
    - `sha256: Option<String>` 64 hex characters, stored lowercase
- `/version/latest`, `/version/latest_batch`, `/version/page`
  - New response fields:
    - `size: Option<i64>`
    - `sha256: Option<String>`
- `/version/page`
  - Versions whose `release_time` is in the future are no longer returned, and `total` no longer counts them, matching `/version/latest`

## 260701

### New

- `/user/following`
- `/user/followers`
- `/user/follow`
- `/user/unfollow`

### Changes

- `/user/profile`
  - New response fields:
    - `follower_count: i64`
    - `following_count: i64`
    - `is_following: Some<bool>`
    - `is_followed_by: Some<bool>`

- `/auth/device/list`
  - New response fields:
    - `token_id: String`
