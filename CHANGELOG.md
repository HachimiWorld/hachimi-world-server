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
- Song comments, two levels deep: comments on a song, and replies under them (a reply to a reply stays under the same top-level comment and names who it replies to). Up to 500 characters each, and up to 30 comments per user per hour. A hidden song's comments are only visible to its uploader
  - `/comment/list` (GET): top-level comments on a song, each with its first 3 replies and `reply_count` (visible replies). Query: `target_type` (`song`), `target_id` (the song id), `sort` (`hot`, most liked first, the default; or `new`), `page_index`, `page_size` (1..=50). Response: `total` (visible comments and replies), `threads`. Comments hidden by a report are hidden from their author too. A deleted or hidden comment that still has visible replies shows with `removed: true` and no content
  - `/comment/replies` (GET): replies under a top-level comment, oldest first. Query: `root_id`, `page_index`, `page_size`. Response: `total`, `replies`
  - `/comment/create` (POST, login): `target_type`, `target_id`, `content`, `reply_to_id` (optional). Response: `comment`, or null when the content check failed and it waits to be checked again, seen by no one, its author included, until it passes. Errors: `invalid_target_type`, `target_not_found`, `comment_not_found`, `comment_empty`, `comment_too_long`, `comment_rejected`, `rate_limited`, `user_banned`
  - `/comment/delete` (POST, login): `comment_id`. For the comment's author or the song's uploader. Errors: `comment_not_found`, `permission_denied`
  - `/comment/like`, `/comment/unlike` (POST, login): `comment_id`
  - Comments are checked like bios, under the subject `song.comment`; without its own policy it uses the text default. A blocked comment isn't saved, and blocked attempts count toward the hourly limit. A comment whose check fails waits in `content_reviews`, kept whole in `payload`, and is checked again by a background worker after 1, 5, 15 and 30 minutes, then hourly. It's saved to `comments` with its id and time once it passes, and dropped if blocked; one that still can't be checked after 24 hours is published. Waiting comments count toward the hourly limit
  - Metrics for held-back content: `content_review_settled_total` (`subject`, `outcome`), and the gauges `content_review_pending` and `content_review_oldest_pending_seconds`. New alerts in `monitoring/alerts/content-check.yaml` fire when content has waited over an hour for its recheck and when content is published unchecked; the content check dashboard shows them
  - `like_count` and `visible_reply_count` on `comments` are derived counts, kept in step in the same transaction as what they count. `cargo run --bin recount_comments` lists any that drifted, and fixes them with `--apply`
  - Comments can be reported (`target_type` `comment`), with the content actions `hide` and `restore`. The author gets `governance.content_hidden` or `governance.content_restored` with `content_intent` `{ action: "song.comment.view", data: { song_display_id, comment_id } }`
- Report targets have a `preview` in their kind's shape, tagged by `type`: `song` (`title`, `cover_url`, `display_id`), `playlist` (`name`, `cover_url`), `user` (`username`, `avatar_url`) or `comment` (`content`, and `on`, the preview of what it's on, or null). `/report/queue` and `/report/case` return it in `target.preview`; `target.title`, `target.cover_url` and `target.display_id` are deprecated
- Message center: replies. Comments replying to the user, and comments on their songs, by others
  - `/message/received_replies` (GET): newest first. Each item has the `comment`, its `target` (`target_type`, `target_id`, `preview`) and `replied_content` (the user's comment it replies to, or null). Query: `before_time` and `before_comment_id` (from the previous page's last comment), `limit` (1..=50, default 20). Response: `items`, `has_more`. Errors with `invalid_cursor`
  - `/message/summary`: new `reply_unread` (counted up to 100) and `reply_read_time`, and `total_unread` includes replies. `/message/mark_read` accepts the channel `reply`
- Reports. Each reported song, playlist or user has one case, reused forever: reports on it merge into the case, and a new report reopens a resolved case unless the last decision chose to ignore further reports. All endpoints require login
  - `/report/submit` (POST): `target_type` (`song`, `playlist`, `user`), `target_id`, `reason` (`spam`, `abuse`, `illegal`, `nsfw`, `copyright`, `other`), `detail` (required for `other`, ≤ 500 chars). Response: `report_id`, `already_reviewed`. Up to 20 reports per user per hour. Errors: `invalid_target_type`, `target_not_found`, `cannot_report_self`, `invalid_reason`, `detail_required`, `detail_too_long`, `already_reported`, `rate_limited`, `user_banned`
  - `/report/queue` (GET, committee and contributors): cases by `status` (`pending` by default, or `resolved`), most recently reported first, with the target, `pending_count` and pending reports per reason. Query: `before_time` and `before_id` (from the previous page's last item), `limit` (1..=50, default 20). Response: `items`, `has_more`
  - `/report/case` (GET, committee and contributors): query `target_type`, `target_id`. Response: `case`, `pending_reports` (with reporters), `actions` (past decisions, newest first, with `content_actions` and `author_reason`), `verdicts` (choosable now), `content_actions` (`{ action, verdict, penalty }` choosable now; penalties need `author_reason`). Errors with `case_not_found`
    - With pending reports: `agree` (when there is a content action to take or the target is already hidden), `disagree`, `ignore`. Without: `disagree` to restore a hidden target, `ignore` to take reports again after ignoring them
    - Content actions are declared by the target's kind: `hide` (song, playlist; a penalty, with `agree`), `restore` (hidden song or playlist; with `disagree`), `reset_avatar`, `reset_bio`, `reset_username` (user; penalties, with `agree`, several at once)
  - `/report/resolve` (POST, contributors): `target_type`, `target_id`, `verdict`, `content_actions`, `author_reason` (shown to the owner, ≤ 500 chars, required for penalties), `note` (internal), `ignore_reports`, `up_to_report_id` (reports after it stay pending). Reporters of the handled reports get `governance.report_resolved`; the owner gets `governance.content_hidden`, `governance.content_restored` or `governance.profile_reset`. Response: `action_id`, `status`, `pending_count`. Errors: `invalid_verdict`, `invalid_content_action`, `content_action_required`, `author_reason_required`, `author_reason_too_long`
  - `/report/hidden_reason` (GET): query `target_type`, `target_id`. For the owner of a song or playlist: `hidden`, `reason`, `hide_time`
  - `/committee/me` (GET): `can_view`, `can_resolve`
  - `/committee/members` (GET, committee and contributors), `/committee/appoint` and `/committee/revoke` (POST, contributors; body `uid`). Errors: `user_not_found`, `already_member`, `not_member`
- Review results now also send the uploader a system notification, in the same transaction as the review: `publish.review_approved`, `publish.review_rejected`, `publish.modify_approved`, `publish.modify_rejected`. `content_intent` is `{ action: "creation.review.view", data: { review_id } }`
- Content checks for usernames and bios, configured by the new **required** `content_check` config (`enabled`, `policy_dir`, `model`, `typesafe.api_key` / `base_url` / `timeout_ms`; see `config/content-check-example/`). Add it before deploying, with `enabled: false` to keep checks off; when false, nothing else in it is read. When enabled, startup tries the key, the model and the policy against TypeSafe: a rejection (such as a bad key or an unknown model) stops the server, while a timeout or an outage only logs a warning. A changed username or non-empty bio is judged by TypeSafe's Jev against the policy; a blocked one isn't saved. Unchanged fields aren't checked. When the check fails, the content passes. The server won't start with a policy whose files don't agree
  - `/user/update_profile` errors: `username_rejected`, `bio_rejected` (the reason isn't given)
  - Playlists are checked too: a new or changed name and a non-empty description on `/playlist/create` and `/playlist/update`, and the cover on `/playlist/set_cover`. The policy needs the subjects `playlist.name`, `playlist.description` and `playlist.cover` (an image), or the server won't start. Errors: `name_rejected`, `description_rejected`, `cover_rejected`; covers share the image limit of 10 checks per user per hour, counted apart from avatars, and error with `rate_limited` past it
  - Every check is recorded in `content_checks`, and each policy version once in `content_policies`
  - Content already judged under the same policy and model (same subject and sha256, by anyone) reuses that answer instead of asking TypeSafe again; the record's `reused_from` names the check it reused. Checks whose call failed aren't reused. A reused image check doesn't count toward the hourly image limit, so a file blocked before is blocked at once even past the limit. A new policy version or model judges everything afresh. Reuses are counted in `content_check_reused_total`
  - A policy can give one default per kind (`defaults.text`, `defaults.image` in `decision.yaml`, with questions in `questions/default.text.json` and `questions/default.image.json`), used by every subject without its own entry under `subjects`. Subjects are now optional when a default covers them; checks and metrics are still recorded per subject
  - Prometheus metrics: `content_check_total{subject, outcome}` (`pass`, `block`, `error`; an error passes the content), `content_check_blocked_total{subject, question}`, `content_check_duration_seconds{subject, outcome}` (a histogram with buckets from 0.05 to 5 seconds), `content_check_input_tokens_total{subject}`, and `content_check_policy_info{version, model}` set to 1 while checks are on

### Changes

- Playlists without a cover uploaded by their owner now show the cover of their most recently added song that isn't hidden, in `cover_url` everywhere a playlist is returned. It's picked again when a song is added or removed; a song hidden or changed later doesn't update it until then. A playlist with no such song still has no cover
  - Deploy only after the release that added `custom_cover` runs on every server. Run the migrations, which mark every cover set before as the owner's, then `mig_261019_fill_playlist_default_covers` (dry run by default, `--apply` to update) to fill in existing playlists
- New `playlists.custom_cover` column, set when the owner uploads a cover with `/playlist/set_cover`. It's the first step of default playlist covers: covers uploaded before it are marked, and default covers turned on, in the next release, once every server sets it
- Register, login, `/auth/refresh_token` and the anonymous play history and recommendation endpoints no longer fail with an internal error when the request has neither `X-Real-IP` nor `CF-Connecting-IP`, for example on a local server without a proxy. They use the connection's IP instead
- The `/api` rate limit counts per user when the request has a valid access token, and per IP otherwise (it was per IP for everyone). It is looser now, 10 requests per second with bursts of 60, and set by the new optional `server.rate_limit` config (`per_second`, `burst_size`)
- Usernames can be longer. `/user/update_profile` now measures width instead of characters: a CJK character (and most emoji) counts as 2 and others as 1, up to 20, so 10 Chinese characters or 20 Latin letters; at most 32 characters. Usernames with control characters such as line breaks, or with nothing visible, are rejected. All errors are still `invalid_username`
- `/user/profile` shows a changed username, bio or avatar right away; before, the cached profile could stay up to 50 minutes old. When updating the search index fails after `/user/update_profile` or `/user/set_avatar` saved, the request no longer errors; the failure is logged
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
