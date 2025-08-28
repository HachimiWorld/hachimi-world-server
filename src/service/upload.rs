use std::io::Cursor;
use bytes::Bytes;
use image::{ImageFormat, ImageReader};
use metrics::{gauge, histogram};
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

/*pub struct UploadedImageTempData {
    pub url: String,
    pub size: usize,
    pub format: String
}

pub async fn get_image_by_temp_key(temp_id: &str) {

}
*/