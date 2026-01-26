use crate::db::post::{Post, PostDao};
use crate::db::CrudDao;
use crate::service::upload::{upload_cover_image_as_temp_id, ImageProcessOptions, ResizeType};
use crate::service::{contributor, upload, user};
use crate::web::jwt::Claims;
use crate::web::result::WebResult;
use crate::web::routes::user::PublicUserProfile;
use crate::web::state::AppState;
use crate::{err, ok};
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Query, State};
use axum::routing::{get, post};
use axum::Router;
use chrono::Utc;
use itertools::Itertools;
use serde::{Deserialize, Serialize};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        // @since 260125 @experimental
        .route("/page", get(page))
        // @since 260125 @experimental
        .route("/detail", get(detail))
        // @since 260125 @experimental
        .route("/create", post(create))
        // @since 260125 @experimental
        .route("/edit", post(edit))
        // @since 260125 @experimental
        .route("/delete", post(delete))
        // @since 260125 @experimental
        .route("/upload_image", post(upload_image).layer(DefaultBodyLimit::max(10 * 1024 * 1024)))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReq {
    pub page_index: i32,
    pub page_size: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageResp {
    pub posts: Vec<PostItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostItem {
    pub id: i64,
    pub author: PublicUserProfile,
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub cover_url: Option<String>,
    pub create_time: chrono::DateTime<Utc>,
    pub update_time: chrono::DateTime<Utc>,
}

#[framed]
pub async fn page(
    state: State<AppState>,
    req: Query<PageReq>,
) -> WebResult<PageResp> {
    let page_index = req.page_index.max(0);
    let page_size = req.page_size.clamp(1, 50);

    let posts = PostDao::page(&state.sql_pool, page_index as i64, page_size as i64).await?;
    let user_ids = posts.iter().map(|p| p.author_uid).collect_vec();
    let users = user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &user_ids).await?;

    let items = posts
        .into_iter()
        .map(|p| PostItem {
            id: p.id,
            author: users.get(&p.author_uid).cloned()
                .unwrap_or_else(|| PublicUserProfile {
                    uid: 0,
                    username: "Unknown".to_string(),
                    avatar_url: None,
                    bio: None,
                    gender: None,
                    is_banned: false,
                }).clone(),
            title: p.title,
            content: "".to_string(),
            content_type: p.content_type,
            cover_url: p.cover_url,
            create_time: p.create_time,
            update_time: p.update_time,
        })
        .collect();

    ok!(PageResp { posts: items })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostIdReq {
    pub post_id: i64,
}

#[framed]
pub async fn detail(
    _claims: Claims,
    state: State<AppState>,
    req: Query<PostIdReq>,
) -> WebResult<PostItem> {
    if let Some(p) = PostDao::get_by_id(&state.sql_pool, req.post_id).await? {
        let user = user::get_public_profile(state.redis_conn.clone(), &state.sql_pool, &[p.author_uid]).await?
            .remove(&p.author_uid)
            .unwrap_or_else(|| PublicUserProfile {
                uid: 0,
                username: "Unknown".to_string(),
                avatar_url: None,
                bio: None,
                gender: None,
                is_banned: false,
            });
        let item = PostItem {
            id: p.id,
            title: p.title,
            content: p.content,
            content_type: p.content_type,
            cover_url: p.cover_url,
            create_time: p.create_time,
            update_time: p.update_time,
            author: user,
        };
        ok!(item)
    } else {
        err!("not_found", "Post not found")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateReq {
    pub title: String,
    pub content: String,
    pub content_type: String,
    pub cover_file_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateResp {
    pub id: i64,
}

#[framed]
pub async fn create(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<CreateReq>,
) -> WebResult<CreateResp> {
    // Validate input
    if req.title.trim().is_empty() || req.title.chars().count() > 200 {
        err!("invalid_title", "Title is invalid")
    }
    if req.content.chars().count() > 10_000 {
        err!("content_too_long", "Content is too long")
    }
    if req.content_type != "markdown" {
        err!("unsupported_content_type", "Unsupported content type")
    }

    // Resolve cover url if provided
    let mut cover_url: Option<String> = None;
    if let Some(ref temp_id) = req.cover_file_id {
        let cover_img = upload::retrieve_from_temp_id(&mut state.redis_conn, "post", &temp_id).await?;
        if let Some(u) = cover_img {
            cover_url = Some(u.url);
        } else {
            err!("invalid_cover_temp_id", "Invalid cover temp id")
        }
    }

    let now = Utc::now();
    let entity = Post {
        id: 0,
        author_uid: claims.uid(),
        title: req.title.clone(),
        content: req.content.clone(),
        content_type: req.content_type.clone(),
        cover_url,
        create_time: now,
        update_time: now,
    };

    let id = PostDao::insert(&state.sql_pool, &entity).await?;
    ok!(CreateResp { id })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditReq {
    pub post_id: i64,
    pub title: Option<String>,
    pub content: Option<String>,
    pub content_type: Option<String>,
    pub cover_file_id: Option<String>,
}

#[framed]
pub async fn edit(
    claims: Claims,
    mut state: State<AppState>,
    req: Json<EditReq>,
) -> WebResult<()> {
    let mut post = if let Some(p) = PostDao::get_by_id(&state.sql_pool, req.post_id).await? { p } else { err!("not_found", "Post not found") };

    // Only author or contributor can edit
    if post.author_uid != claims.uid() {
        contributor::ensure_contributor(&state, claims.uid()).await?;
    }

    if let Some(ref t) = req.title {
        if t.trim().is_empty() || t.chars().count() > 200 {
            err!("invalid_title", "Title is invalid")
        }
        post.title = t.clone();
    }
    if let Some(ref c) = req.content {
        if c.chars().count() > 10_000 {
            err!("content_too_long", "Content is too long")
        }
        post.content = c.clone();
    }

    if let Some(ref temp_id) = req.cover_file_id {
        let cover_img = upload::retrieve_from_temp_id(&mut state.redis_conn, "post", &temp_id).await?;
        if let Some(u) = cover_img {
            post.cover_url = Some(u.url);
        } else {
            err!("invalid_cover_temp_id", "Invalid cover temp id")
        }
    }

    post.update_time = Utc::now();
    PostDao::update_by_id(&state.sql_pool, &post).await?;

    ok!(())
}

#[framed]
pub async fn delete(
    claims: Claims,
    state: State<AppState>,
    req: Json<PostIdReq>,
) -> WebResult<()> {
    // Only contributors can delete posts (keep existing behavior)
    contributor::ensure_contributor(&state, claims.uid()).await?;

    // Ensure exists
    if PostDao::get_by_id(&state.sql_pool, req.post_id).await?.is_none() {
        err!("not_found", "Post not found")
    }

    PostDao::delete_by_id(&state.sql_pool, req.post_id).await?;

    ok!(() )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadImageResp {
    pub file_id: String,
}

#[framed]
pub async fn upload_image(
    claims: Claims,
    state: State<AppState>,
    multipart: Multipart,
) -> WebResult<UploadImageResp> {
    contributor::ensure_contributor(&state, claims.uid()).await?;

    let file_id = upload_cover_image_as_temp_id(
        "post",
        state,
        multipart,
        10 * 1024 * 1024,
        ImageProcessOptions {
            max_width: 512,
            max_height: 512,
            resize_type: ResizeType::Fit,
            quality: 85f32,
        }).await?;

    ok!(UploadImageResp { file_id })
}