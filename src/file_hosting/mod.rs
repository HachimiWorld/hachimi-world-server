use anyhow::Context;
use async_trait::async_trait;
use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use mockall::automock;
use tracing::info;


#[async_trait]
#[automock]
pub trait FileHost: Send + Sync {
    async fn upload(&self, bytes: Bytes, key: &str) -> anyhow::Result<UploadResult>;
    async fn rename(&self, old_key: &str, new_key: &str) -> anyhow::Result<()>;
}

pub struct S3FileHost {
    bucket_name: String,
    client: aws_sdk_s3::Client,
    public_domain: String,
}


impl S3FileHost {
    pub fn new(bucket_name: String, client: aws_sdk_s3::Client, public_domain: String) -> Self {
        S3FileHost {
            bucket_name,
            client,
            public_domain,
        }
    }
}

#[async_trait]
impl FileHost for S3FileHost {
    async fn upload(&self, bytes: Bytes, key: &str) -> anyhow::Result<UploadResult> {
        info!("Uploading file {} to r2. Total: {} bytes", key, bytes.len());
        let body = ByteStream::from(bytes);
        let result = self
            .client
            .put_object()
            .bucket(self.bucket_name.clone())
            .body(body)
            .key(key)
            .set_content_type(content_type_for_key(key).map(str::to_string))
            .send()
            .await
            .with_context(|| format!("Failed to upload {}", key))?;
        let url = format!("https://{}/{}", self.public_domain, key);
        info!("Uploaded to {}", url);
        Ok(UploadResult {
            output: result,
            public_url: url,
        })
    }

    async fn rename(&self, old_key: &str, new_key: &str) -> anyhow::Result<()> {
        self.client
            .copy_object()
            .bucket(self.bucket_name.clone())
            .copy_source(format!("/{}/{}", self.bucket_name, old_key))
            .key(new_key)
            .send()
            .await
            .with_context(|| format!("Failed to rename {}", old_key))?;
        Ok(())
    }
}

/// Content-Type to store with an object, guessed from its key's extension. Without one, R2 serves
/// the object as `application/octet-stream`, and search engines may not treat cover images as
/// images. Returns `None` for extensions we don't recognize, so the object is stored as before.
pub fn content_type_for_key(key: &str) -> Option<&'static str> {
    let ext = key.rsplit_once('.')?.1.to_ascii_lowercase();
    let content_type = match ext.as_str() {
        "webp" => "image/webp",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "avif" => "image/avif",
        "gif" => "image/gif",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "xml" => "application/xml",
        _ => return None,
    };
    Some(content_type)
}

pub struct UploadResult {
    pub output: PutObjectOutput,
    pub public_url: String,
}

#[cfg(test)]
mod tests {
    use super::content_type_for_key;

    #[test]
    fn content_type_from_extension() {
        assert_eq!(content_type_for_key("images/cover/abc.webp"), Some("image/webp"));
        assert_eq!(content_type_for_key("images/cover/abc.JPG"), Some("image/jpeg"));
        assert_eq!(content_type_for_key("songs/abc.flac"), Some("audio/flac"));
        assert_eq!(content_type_for_key("sitemap/secret/index.xml"), Some("application/xml"));
        assert_eq!(content_type_for_key("songs/abc.aac"), None);
        assert_eq!(content_type_for_key("no-extension"), None);
    }
}
