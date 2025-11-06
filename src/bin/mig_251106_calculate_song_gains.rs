use hachimi_world_server::audio;
use hachimi_world_server::config::Config;
use serde::Deserialize;
use std::io::Write;
use std::{env, fs};
use tokio::time::Instant;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let cfg = Config::parse(&env::var("MIG_CALCULATE_SONG_GAINS_CONFIG_PATH").unwrap()).unwrap();
    let db_cfg: DatabaseConfig = cfg.get_and_parse("db").unwrap();
    let sql_pool = sqlx::PgPool::connect(&format!("postgres://{}:{}@{}/{}", db_cfg.username, db_cfg.password, db_cfg.address, db_cfg.database)).await.unwrap();

    let mut tx = sql_pool.begin().await.unwrap();
    let songs = sqlx::query!("SELECT id, display_id, title, file_url, gain FROM songs WHERE gain IS NULL")
        .fetch_all(&mut *tx)
        .await.unwrap();
    let len = songs.len();
    for (i, x) in songs.iter().enumerate() {
        println!("Processing({i}/{len}) {} - {}, {}", x.display_id, x.title, x.file_url);
        let start = Instant::now();
        fs::create_dir_all("temp_download").unwrap();
        let temp_file = format!("temp_download/{}.{}", x.display_id, x.file_url.rsplit_once('.').unwrap().1);

        println!("Downloading file to {}", temp_file);
        if fs::exists(&temp_file).unwrap() {
            println!("File already exists, skipping download and processing. {}", x.display_id);
        } else {
            let bytes = reqwest::get(&x.file_url).await.unwrap().bytes().await.unwrap();
            fs::File::create(&temp_file).unwrap().write_all(&bytes).unwrap();
        };
        let metadata = audio::parse_and_validate(Box::new(fs::File::open(temp_file).unwrap()), Some(x.file_url.as_str())).unwrap();

        println!("Processing time: {:?}, gain: {}", start.elapsed(), metadata.gain_db);
        sqlx::query!("UPDATE songs SET gain = $1 WHERE id = $2", metadata.gain_db, x.id).execute(&mut *tx).await.unwrap();
    }
    tx.commit().await.unwrap();
}

#[derive(Deserialize, Clone, Debug)]
struct DatabaseConfig {
    pub address: String,
    pub username: String,
    pub password: String,
    pub database: String,
}