use std::io::Cursor;
use anyhow::{anyhow, Context};
use async_backtrace::framed;
use axum::extract::{DefaultBodyLimit, Multipart, State};
use axum::Router;
use axum::routing::post;
use image::{ImageFormat, ImageReader};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use crate::{common, err, ok};
use crate::web::jwt::Claims;
use crate::web::result::{WebResult};
use crate::web::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/upload_image", post(upload_image))
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadImageResp {
    pub temp_id: String,
}

#[framed]
async fn upload_image(
    claims: Claims,
    mut state: State<AppState>,
    mut multipart: Multipart,
) -> WebResult<UploadImageResp> {
    let data_field = multipart.next_field().await?.with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    let start_time = std::time::Instant::now();

    // Validate image
    if bytes.len() > 8 * 1024 * 1024 {
        err!("image_too_large", "Image size must be less than 8MB");
    }
    let format = ImageReader::new(Cursor::new(bytes.clone()))
        .with_guessed_format()
        .map_err(|_| common!("invalid_image", "Invalid image"))?
        .format()
        .ok_or_else(|| common!("invalid_image", "Invalid image"))?;

    let format_ext = match format {
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Avif => {
            format.extensions_str().first().ok_or_else(|| anyhow!("Cannot get extension name"))?
        }
        _ => err!("format_unsupported", "Image format unsupported")
    };

    // Upload image
    let sha1 = openssl::sha::sha1(&bytes);
    let filename = format!("images/cover/{}.{}", hex::encode(sha1), format_ext);
    let result = state.file_host.upload(bytes, &filename).await?;
    let temp_id = uuid::Uuid::new_v4().to_string();
    
    let _: () = state.redis_conn.set_ex(build_image_temp_key(&temp_id), result.public_url, 3600).await?;

    // Add metrics
    let duration = start_time.elapsed();
    let histogram = metrics::histogram!("upload_image_duration_secs");
    histogram.record(duration.as_secs_f64());
    

    ok!(UploadImageResp { temp_id })
}

fn build_image_temp_key(temp_id: &str) -> String {
    let key = format!("upload:image:{}", temp_id);
    key
}