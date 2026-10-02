# Changelog

## Unreleased

### New

- Daily sitemap generation, enabled by the new optional `sitemap` config (`site_url`, `path_secret`). Uploads `sitemap/<path_secret>/index.xml` and `songs-<n>.xml` to s3; hachimi.world serves them under `/sitemap/<path_secret>/`.

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
