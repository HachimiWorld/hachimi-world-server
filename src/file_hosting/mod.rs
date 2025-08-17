use aws_sdk_s3::operation::put_object::PutObjectOutput;
use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;

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
        let body = ByteStream::from(bytes);
        let result = self
            .client
            .put_object()
            .bucket(self.bucket_name.clone())
            .body(body)
            .key(key)
            .send()
            .await?;
        let url = format!("https://{}/{}", self.public_domain, key);
        Ok(UploadResult {
            output: result,
            public_url: url,
        })
    }
}

pub struct UploadResult {
    pub output: PutObjectOutput,
    pub public_url: String,
}
