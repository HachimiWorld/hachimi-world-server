use std::io::Cursor;
use std::time::Instant;
use bytes::Bytes;
use image::{ImageFormat, ImageReader};
use image::imageops::FilterType;
use metrics::{histogram};
use tracing::{info};
use crate::service::upload::ValidationError::{InvalidImage, UnsupportedFormat};

#[derive(thiserror::Error, Debug)]
pub enum ValidationError {
    #[error("invalid image")]
    InvalidImage,
    #[error("the image format is unsupported")]
    UnsupportedFormat
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

    let webp_encoder = webp::Encoder::from_image(&resized).unwrap();
    let webp = webp_encoder.encode(quality);

    info!("Image scale down to webp took {:?}, size from {} to {}", start.elapsed(), len, webp.len());
    histogram!("image_scale_down_to_webp_duration_secs").record(start.elapsed().as_secs_f64());
    Ok(webp.to_vec())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use crate::service::upload::{scale_down_to_webp, ResizeType};

    #[test]
    fn test_scale_down() {
        let bytes = fs::read(".local/test_res/test.png").unwrap();
        let webp = scale_down_to_webp(1920, 1920, bytes.into(), ResizeType::Fit, 95f32).unwrap();
    }
}
/*pub struct UploadedImageTempData {
    pub url: String,
    pub size: usize,
    pub format: String
}

pub async fn get_image_by_temp_key(temp_id: &str) {

}
*/