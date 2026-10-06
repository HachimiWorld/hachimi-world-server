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
- System notifications. Producers call `service::notification::send_notification` inside their own transaction. Notifications older than 180 days are hidden and deleted hourly. All endpoints require login, and `notification_id` is a UUIDv7 string
  - Notification fields: `notification_id`, `type`, `title`, `body`, `content_intent` (`{ action, data }` or null), `read_time` (null if unread), `create_time`
  - `/notification/list` (GET): newest first. Query: `before_id` (last id of the previous page), `limit` (1..=50, default 20). Response: `items`, `has_more`
  - `/notification/detail` (GET): query `notification_id`. Doesn't mark it read
  - `/notification/unread_count` (GET): `unread_count`
  - `/notification/mark_read` (POST): body `notification_id`. Response: `notification_id`, `read_time` (the first read time), `unread_count`
  - `/notification/mark_all_read` (POST): response `marked_count`, `unread_count`
  - Errors: `invalid_notification_id`, `notification_unavailable` (not yours, doesn't exist or expired)
- Review results now also send the uploader a system notification, in the same transaction as the review: `publish.review_approved`, `publish.review_rejected`, `publish.modify_approved`, `publish.modify_rejected`. `content_intent` is `{ action: "creation.review.view", data: { review_id } }`

### Changes

- Notification emails (review results, new submissions, review comments and submission updates) go through a transactional outbox: they are written in the same transaction as the change and sent by a background relay, with retries for up to 8 attempts. A mail failure no longer fails the request, and an email is never sent for a change that was rolled back. Verification code emails are still sent directly
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
