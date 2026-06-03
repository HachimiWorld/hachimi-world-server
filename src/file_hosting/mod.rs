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

pub struct UploadResult {
    pub output: PutObjectOutput,
    pub public_url: String,
}
