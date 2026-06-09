use crate::common;
use crate::service::upload::ValidationError::{InvalidImage, UnsupportedFormat};
use crate::web::result::{CommonError, WebError};
use crate::web::state::AppState;
use anyhow::{anyhow, Context};
use axum::extract::{Multipart, State};
use bytes::Bytes;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, ImageReader};
use metrics::histogram;
use redis::aio::ConnectionManager;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::time::Instant;
use tracing::info;

#[derive(thiserror::Error, Debug)]
pub enum ValidationError {
    #[error("invalid image")]
    InvalidImage,
    #[error("the image format is unsupported")]
    UnsupportedFormat,
}

pub async fn validate_image_and_get_ext<'a>(bytes: Bytes) -> Result<&'a str, ValidationError> {
    let start = std::time::Instant::now();

    let format = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| InvalidImage)?
        .format()
        .ok_or_else(|| InvalidImage)?;

    let format_ext = match format {
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP | ImageFormat::Avif => {
            format.extensions_str().first().ok_or_else(|| UnsupportedFormat)?
        }
        _ => Err(UnsupportedFormat)?
    };

    histogram!("image_validation_duration_secs").record(start.elapsed().as_secs_f64());

    Ok(format_ext)
}

#[derive(Debug, Copy, Clone)]
pub enum ResizeType {
    Crop, Fit, Exact
}

pub fn scale_down_to_webp(
    w: u32,
    h: u32,
    bytes: Bytes,
    resize_type: ResizeType,
    quality: f32
) -> anyhow::Result<Vec<u8>> {
    let start = Instant::now();
    let len = bytes.len();
    let image = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    // Convert to rgb8 if needed because the webp encoder only supports rgb8/rgba8
    let image = match image {
        x @ DynamicImage::ImageRgb8(_) => x,
        x @ DynamicImage::ImageRgba8(_) => x,
        x => {
            info!("Converting image to rgb8");
            DynamicImage::ImageRgb8(x.into_rgb8())
        }
    };
    // Resize image
    let resized = if image.width() > w || image.height() > h {
        match resize_type {
            ResizeType::Crop => image.resize_to_fill(w, h, FilterType::Lanczos3),
            ResizeType::Fit => image.resize(w, h, FilterType::Lanczos3),
            ResizeType::Exact => image.resize(w, h, FilterType::Lanczos3)
        }
    } else {
        image
    };

    let webp_encoder = webp::Encoder::from_image(&resized).map_err(|_| anyhow!("Failed to encode image to webp"))?;
    let webp = webp_encoder.encode(quality);

    info!("Image scale down to webp took {:?}, size from {} to {}", start.elapsed(), len, webp.len());
    histogram!("image_scale_down_to_webp_duration_secs").record(start.elapsed().as_secs_f64());
    Ok(webp.to_vec())
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedImageTempData {
    pub module_type: String,
    pub url: String,
    pub size: usize,
    pub format: String
}

#[derive(Debug)]
pub struct ImageProcessOptions {
    pub max_width: u32,
    pub max_height: u32,
    pub resize_type: ResizeType,
    pub quality: f32,
}

impl ImageProcessOptions {
    pub fn default_cover() -> Self {
        ImageProcessOptions {
            max_width: 1024,
            max_height: 1024,
            resize_type: ResizeType::Fit,
            quality: 90f32,
        }
    }

    pub fn default_avatar() -> Self {
        ImageProcessOptions {
            max_width: 256,
            max_height: 256,
            resize_type: ResizeType::Crop,
            quality: 80f32,
        }
    }
}

pub async fn upload_cover_image_as_temp_id(
    module_type: &str,
    mut state: State<AppState>,
    mut multipart: Multipart,
    max_size: usize,
    options: ImageProcessOptions,
) -> Result<String, WebError<CommonError>> {
    let data_field = multipart
        .next_field()
        .await?
        .with_context(|| "No data field found")?;
    let bytes = data_field.bytes().await?;

    // Validate image
    if bytes.len() > max_size {
        Err(common!("image_too_large", "Image size too large"))?;
    }

    let webp = scale_down_to_webp(options.max_width, options.max_height, bytes.clone(), options.resize_type, options.quality)
        .map_err(|_| common!("invalid_image", "The image is not supported"))?;

    // Upload image
    let sha1 = openssl::sha::sha1(&webp);
    let filename = format!("images/{}/{}.webp", module_type, hex::encode(sha1));
    let result = state.file_host.upload(webp.into(), &filename).await?;
    let temp_id = uuid::Uuid::new_v4().to_string();
    let temp_data = UploadedImageTempData {
        module_type: module_type.to_string(),
        url: result.public_url,
        size: bytes.len(),
        format: "webp".to_string(),
    };
    let temp_data_json = serde_json::to_string(&temp_data)?;

    let _: () = state.redis_conn
        .set_ex(build_image_temp_key(module_type, &temp_id), temp_data_json, 3600)
        .await?;

    Ok(temp_id)
}

pub async fn retrieve_from_temp_id(
    redis: &mut ConnectionManager,
    module_type: &str,
    temp_id: &str,
) -> anyhow::Result<Option<UploadedImageTempData>> {
    let key = build_image_temp_key(module_type, temp_id);
    let value: Option<String> = redis.get(key).await?;
    if let Some(v) = value {
        let data: UploadedImageTempData = serde_json::from_str(&v)?;
        Ok(Some(data))
    } else {
        Ok(None)
    }
}

fn build_image_temp_key(module_type: &str, temp_id: &str) -> String {
    let key = format!("upload:image:{}:{}", module_type, temp_id);
    key
}

#[cfg(test)]
mod tests {
    use crate::service::upload::{scale_down_to_webp, ResizeType};
    use image::{DynamicImage, ImageBuffer, ImageFormat, Rgb};
    use std::io::Cursor;

    /// Generate a test image in any format as raw bytes.
    fn make_test_image(w: u32, h: u32, format: ImageFormat) -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, _> = ImageBuffer::from_fn(w, h, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut buf = Vec::new();
        img.write_to(&mut Cursor::new(&mut buf), format).unwrap();
        buf
    }

    #[test]
    fn test_scale_down_no_resize_needed() {
        let png = make_test_image(100, 100, ImageFormat::Png);
        let webp = scale_down_to_webp(200, 200, png.into(), ResizeType::Fit, 90.0).unwrap();

        // Output should be valid WebP (starts with "RIFF" magic)
        assert!(webp.len() > 12);
        assert_eq!(&webp[0..4], b"RIFF");
        assert_eq!(&webp[8..12], b"WEBP");
    }

    #[test]
    fn test_scale_down_resize_triggered() {
        let png = make_test_image(800, 600, ImageFormat::Png);
        let webp = scale_down_to_webp(400, 300, png.into(), ResizeType::Fit, 90.0).unwrap();

        assert!(webp.len() > 12);
        assert_eq!(&webp[0..4], b"RIFF");
    }

    #[test]
    fn test_scale_down_rgb48be() {
        // 16-bit deep color images must be converted to RGB8 first
        let img = DynamicImage::ImageRgb16(
            ImageBuffer::from_fn(64, 64, |x, y| {
                image::Rgb([((x * 256) % 65536) as u16, ((y * 256) % 65536) as u16, 32768])
            })
        );
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png).unwrap();

        let webp = scale_down_to_webp(128, 128, png.into(), ResizeType::Fit, 95.0).unwrap();
        assert_eq!(&webp[0..4], b"RIFF");
    }

    #[test]
    fn test_scale_down_quality_affects_size() {
        let png = make_test_image(400, 300, ImageFormat::Png);
        let low_q  = scale_down_to_webp(400, 300, png.clone().into(), ResizeType::Fit, 10.0).unwrap();
        let high_q = scale_down_to_webp(400, 300, png.into(), ResizeType::Fit, 99.0).unwrap();

        // Lower quality should produce smaller output
        assert!(low_q.len() < high_q.len());
    }

    #[test]
    fn test_scale_down_invalid_bytes() {
        let result = scale_down_to_webp(100, 100, vec![0u8; 10].into(), ResizeType::Fit, 90.0);
        assert!(result.is_err());
    }
}