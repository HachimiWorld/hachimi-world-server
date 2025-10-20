use std::env;
use aws_sdk_s3::config::Region;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use hachimi_world_server::config::Config;
use hachimi_world_server::file_hosting::FileHost;
use hachimi_world_server::service::upload::{scale_down_to_webp, ResizeType};

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let cfg = Config::parse(&env::var("COMPRESS_SONG_COVERS_CONFIG_PATH").unwrap()).unwrap();
    let db_cfg: DatabaseConfig = cfg.get_and_parse("db").unwrap();
    let file_host = get_file_host(cfg).await.unwrap();
    let sql_pool = sqlx::PgPool::connect(&format!("postgres://{}:{}@{}/{}", db_cfg.username, db_cfg.password, db_cfg.address, db_cfg.database)).await.unwrap();

    let mut tx = sql_pool.begin().await.unwrap();
    let songs = sqlx::query!("SELECT id, display_id, title, cover_art_url FROM songs")
        .fetch_all(&mut *tx)
        .await.unwrap();
    let len = songs.len();
    for (i, x) in songs.iter().enumerate() {
        if !x.cover_art_url.ends_with(".webp") {
            println!("Processing({i}/{len}) {} - {}, {}", x.display_id, x.title, x.cover_art_url);
            let start = Instant::now();
            let bytes = reqwest::get(&x.cover_art_url).await.unwrap().bytes().await.unwrap();
            let origin_size = bytes.len();
            let data = scale_down_to_webp(1024, 1024, bytes, ResizeType::Fit, 90f32).unwrap();
            let sha1 = openssl::sha::sha1(&data);
            let filename = format!("images/cover/{}.webp", hex::encode(sha1));
            let bytes = bytes::Bytes::from(data);
            println!("Compress from {}bytes to {}bytes in {:?}.", origin_size, bytes.len(), start.elapsed());

            let upload_result = file_host.upload(bytes.clone(), &filename).await.unwrap();
            println!("Uploaded to {}", upload_result.public_url);

            let result = file_host.upload(bytes, &filename).await.unwrap();
            sqlx::query!("UPDATE songs SET cover_art_url = $1 WHERE id = $2", &result.public_url, x.id).execute(&mut *tx).await.unwrap();
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
struct DatabaseConfig {
    pub address: String,
    pub username: String,
    pub password: String,
    pub database: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct S3Config {
    pub bucket_name: String,
    pub endpoint_url: String,
    pub public_domain: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}
async fn get_file_host(config: Config) -> anyhow::Result<FileHost> {
    let cfg: S3Config = config.get_and_parse("s3")?;

    // Configure the client
    let config = aws_sdk_s3::Config::builder()
        .endpoint_url(cfg.endpoint_url)
        .credentials_provider(aws_sdk_s3::config::Credentials::new(
            cfg.access_key_id,
            cfg.access_key_secret,
            None, // session token is not used with R2
            None,
            "R2",
        ))
        .region(Region::new("auto"))
        .behavior_version_latest()
        .build();

    let client = aws_sdk_s3::Client::from_conf(config);
    Ok(FileHost::new(
        cfg.bucket_name,
        cfg.public_domain,
        client,
    ))
}