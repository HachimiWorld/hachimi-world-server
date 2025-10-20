use anyhow::Context;
use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use tracing::info;

pub struct FileHost {
    bucket_name: String,
    client: aws_sdk_s3::Client,
    public_domain: String,
}

impl FileHost {
    pub fn new(bucket_name: String, public_domain: String, client: aws_sdk_s3::Client) -> Self {
        FileHost {
            bucket_name,
            public_domain,
            client,
        }
    }

    pub async fn upload(&self, bytes: Bytes, key: &str) -> anyhow::Result<UploadResult> {
        info!("Uploading file {} to r2. Total: {} bytes", key, bytes.len());
        let body = ByteStream::from(bytes);
        let result = self
            .client
            .put_object()
            .bucket(self.bucket_name.clone())
            .body(body)
            .key(key)
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

    pub async fn rename(&self, old_key: &str, new_key: &str) -> anyhow::Result<()> {
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

pub struct UploadResult {
    pub output: PutObjectOutput,
    pub public_url: String,
}
