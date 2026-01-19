use crate::db::song_tag::{ISongTagDao, SongTag, SongTagDao};
use chrono::{DateTime, Utc};
use itertools::Itertools;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct RecommendTagQuery {
    pub user_id: i64,
    pub limit: i64,
    pub since: DateTime<Utc>,
}

/// Recommend tags for a user, based on the tags of songs they played recently.
///
/// Heuristic:
/// - Count tag occurrences from the user's play history since `since`
/// - Order by count desc, then tag_id asc
/// - Return up to `limit` tags
pub async fn recommend_tags_by_play_history(
    pool: &PgPool,
    query: RecommendTagQuery,
) -> anyhow::Result<Vec<(SongTag, i64)>> {
    let limit = query.limit.clamp(1, 50);

    // tag_id -> count
    let rows = sqlx::query!(
        r#"
        SELECT str.tag_id AS tag_id, COUNT(*)::bigint AS cnt
        FROM song_plays sp
        JOIN song_tag_refs str ON sp.song_id = str.song_id
        WHERE sp.user_id = $1 AND sp.create_time >= $2
        GROUP BY str.tag_id
        ORDER BY cnt DESC, str.tag_id ASC
        LIMIT $3
        "#,
        query.user_id,
        query.since,
        limit,
    )
    .fetch_all(pool)
    .await?;

    let tag_ids = rows.iter().map(|r| r.tag_id).collect_vec();
    let tags = SongTagDao::list_by_ids(pool, &tag_ids).await?;

    let tag_map = tags.into_iter().map(|t| (t.id, t)).collect::<std::collections::HashMap<_, _>>();

    let result = rows
        .into_iter()
        .filter_map(|r| tag_map.get(&r.tag_id).cloned().map(|t| (t, r.cnt.unwrap_or(0))))
        .collect_vec();

    Ok(result)
}

/// Recommend tags by combining play-history and liked-song tags.
///
/// Currently:
/// - Plays since `since` (weight 1.0)
/// - Liked songs ever (weight 0.5)
pub async fn recommend_tags(
    pool: &PgPool,
    user_id: i64,
    limit: i64,
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<(SongTag, i64)>> {
    let limit = limit.clamp(1, 50);

    // Plays: tag_id -> count
    let play_rows = sqlx::query!(
        r#"
        SELECT str.tag_id AS tag_id, COUNT(*)::bigint AS cnt
        FROM song_plays sp
        JOIN song_tag_refs str ON sp.song_id = str.song_id
        WHERE sp.user_id = $1 AND sp.create_time >= $2
        GROUP BY str.tag_id
        "#,
        user_id,
        since,
    )
    .fetch_all(pool)
    .await?;

    // Likes: tag_id -> count of liked songs containing tag
    let like_rows = sqlx::query!(
        r#"
        SELECT str.tag_id AS tag_id, COUNT(*)::bigint AS cnt
        FROM song_likes sl
        JOIN song_tag_refs str ON sl.song_id = str.song_id
        WHERE sl.user_id = $1
        GROUP BY str.tag_id
        "#,
        user_id,
    )
    .fetch_all(pool)
    .await?;

    // Combine with weights: score = play_cnt*2 + like_cnt
    let mut score_map: std::collections::HashMap<i64, i64> = std::collections::HashMap::new();
    for r in play_rows {
        let cnt = r.cnt.unwrap_or(0);
        *score_map.entry(r.tag_id).or_insert(0) += cnt * 2;
    }
    for r in like_rows {
        let cnt = r.cnt.unwrap_or(0);
        *score_map.entry(r.tag_id).or_insert(0) += cnt;
    }

    let mut tag_ids = score_map.keys().copied().collect_vec();
    // Deterministic final ordering:
    tag_ids.sort();

    let tags = SongTagDao::list_by_ids(pool, &tag_ids).await?;
    let tag_map = tags.into_iter().map(|t| (t.id, t)).collect::<std::collections::HashMap<_, _>>();

    let mut scored = score_map
        .into_iter()
        .filter_map(|(tag_id, score)| tag_map.get(&tag_id).cloned().map(|t| (t, score)))
        .collect_vec();

    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.id.cmp(&b.0.id)));
    scored.truncate(limit as usize);

    Ok(scored)
}


/// Get hot tags based on the previous 10k play history
pub async fn get_hot_tags(
    pool: &PgPool,
    history_limit: i64,
    tag_limit: i64,
) -> anyhow::Result<Vec<(SongTag, i64)>> {
    let play_rows = sqlx::query!(
        r#"
        WITH latest_plays AS (
            SELECT id, song_id
            FROM song_plays
            ORDER BY create_time DESC
            LIMIT $1
        )
        SELECT str.tag_id, COUNT(*)::bigint AS cnt
        FROM latest_plays sp JOIN song_tag_refs str ON sp.song_id = str.song_id
        GROUP BY str.tag_id
        ORDER BY cnt DESC LIMIT $2;
        "#,
        history_limit,
        tag_limit
    ).fetch_all(pool).await?;

    let tag_ids = play_rows.iter().map(|r| r.tag_id).collect_vec();
    let tags = SongTagDao::list_by_ids(pool, &tag_ids).await?;
    let tag_map = tags.into_iter().map(|t| (t.id, t)).collect::<std::collections::HashMap<_, _>>();
    let result = play_rows.into_iter()
        .filter_map(|r| tag_map.get(&r.tag_id).cloned().map(|t| (t, r.cnt.unwrap_or(0))))
        .collect_vec();
    Ok(result)
}