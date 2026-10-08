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
- Message center for received likes and new followers. Each keeps one read time per user and kind; opening it marks everything up to now read. Before the first read, the last 30 days count as unread. All endpoints require login
  - `/message/summary` (GET): `total_unread`, `system_unread`, `like_unread`, `follow_unread`, `like_read_time`, `follow_read_time`
  - `/message/mark_read` (POST): body `channel` (`like` or `follow`). Response: `read_time`. Errors with `invalid_channel`
  - `/message/received_likes` (GET): the user's songs liked by others, most recently liked first, with `like_count`, `latest_like_time` and up to 3 `latest_likers`. Query: `before_time` and `before_song_id` (from the previous page's last item), `limit` (1..=50, default 20). Response: `items`, `has_more`. Errors with `invalid_cursor`
- Reports. Each reported song, playlist or user has one case, reused forever: reports on it merge into the case, and a new report reopens a resolved case unless the last decision chose to ignore further reports. All endpoints require login
  - `/report/submit` (POST): `target_type` (`song`, `playlist`, `user`), `target_id`, `reason` (`spam`, `abuse`, `illegal`, `nsfw`, `copyright`, `other`), `detail` (required for `other`, ≤ 500 chars). Response: `report_id`, `already_reviewed`. Up to 20 reports per user per hour. Errors: `invalid_target_type`, `target_not_found`, `cannot_report_self`, `invalid_reason`, `detail_required`, `detail_too_long`, `already_reported`, `rate_limited`, `user_banned`
  - `/report/queue` (GET, committee and contributors): cases by `status` (`pending` by default, or `resolved`), most recently reported first, with the target, `pending_count` and pending reports per reason. Query: `before_time` and `before_id` (from the previous page's last item), `limit` (1..=50, default 20). Response: `items`, `has_more`
  - `/report/case` (GET, committee and contributors): query `target_type`, `target_id`. Response: `case`, `pending_reports` (with reporters), `actions` (past decisions, newest first, with `content_actions` and `author_reason`), `verdicts` (choosable now), `content_actions` (`{ action, verdict }` choosable now). Errors with `case_not_found`
    - With pending reports: `agree` (when there is a content action to take or the target is already hidden), `disagree`, `ignore`. Without: `disagree` to restore a hidden target, `ignore` to take reports again after ignoring them
    - Content actions: `hide` (song, playlist; with `agree`), `restore` (hidden song or playlist; with `disagree`), `reset_avatar`, `reset_bio`, `reset_username` (user; with `agree`, several at once)
  - `/report/resolve` (POST, contributors): `target_type`, `target_id`, `verdict`, `content_actions`, `author_reason` (shown to the owner, ≤ 500 chars, required for penalties), `note` (internal), `ignore_reports`, `up_to_report_id` (reports after it stay pending). Reporters of the handled reports get `governance.report_resolved`; the owner gets `governance.content_hidden`, `governance.content_restored` or `governance.profile_reset`. Response: `action_id`, `status`, `pending_count`. Errors: `invalid_verdict`, `invalid_content_action`, `content_action_required`, `author_reason_required`, `author_reason_too_long`
  - `/report/owner_notice` (GET): query `target_type`, `target_id`. For the owner of a song or playlist: `hidden`, `reason`, `hide_time`
  - `/committee/me` (GET): `can_view`, `can_resolve`
  - `/committee/members` (GET, committee and contributors), `/committee/appoint` and `/committee/revoke` (POST, contributors; body `uid`). Errors: `user_not_found`, `already_member`, `not_member`
- Review results now also send the uploader a system notification, in the same transaction as the review: `publish.review_approved`, `publish.review_rejected`, `publish.modify_approved`, `publish.modify_rejected`. `content_intent` is `{ action: "creation.review.view", data: { review_id } }`

### Changes

- Songs and playlists can be hidden by a report decision (`is_hidden`). A hidden one is only visible to its owner: `/song/detail` and `/song/detail_by_id` return `not_found` to others (send the token to get your own), and it is left out of search, lists, recommendations and the sitemap. Approving a modification of a hidden song shows it again
  - `/playlist/detail`: new `unavailable_songs` (`song_id`, `order_index`, `add_time`) for songs that were deleted or hidden; `playlist_info.is_hidden`; `songs_count` now counts them too
  - `/song/likes/page_my_likes`: new `unavailable` (`song_id`, `liked_time`) for liked songs that were deleted or hidden
  - `PublicSongDetail.is_hidden`, `PlaylistItem.is_hidden`
- `/publish/review/approve`, `/publish/change_jmid`
  - Failing to update the search index or clear song caches after the change is saved is logged instead of failing the request
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
