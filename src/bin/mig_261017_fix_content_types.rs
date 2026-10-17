//! Sets the Content-Type of objects uploaded before `FileHost::upload` started setting it. Those
//! objects are served as `application/octet-stream`, which may stop search engines from treating
//! cover images as images.
//!
//! Dry run by default: lists what would change. Pass `--apply` to rewrite the objects in place.
//! Only objects whose extension `content_type_for_key` recognizes are touched.

use aws_sdk_s3::config::Region;
use aws_sdk_s3::types::MetadataDirective;
use hachimi_world_server::config::Config;
use hachimi_world_server::file_hosting::content_type_for_key;
use serde::{Deserialize, Serialize};
use std::env;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();
    let apply = env::args().any(|x| x == "--apply");
    let cfg = Config::parse(&env::var("FIX_CONTENT_TYPES_CONFIG_PATH").unwrap()).unwrap();
    let s3_cfg: S3Config = cfg.get_and_parse("s3").unwrap();
    let bucket = s3_cfg.bucket_name.clone();
    let client = get_client(s3_cfg);

    if !apply {
        println!("Dry run. Pass --apply to change the objects.");
    }

    let (mut scanned, mut skipped, mut correct, mut fixed) = (0, 0, 0, 0);
    let mut pages = client.list_objects_v2().bucket(&bucket).into_paginator().send();
    while let Some(page) = pages.next().await {
        let page = page.unwrap();
        for object in page.contents() {
            let Some(key) = object.key() else { continue };
            scanned += 1;
            let Some(expected) = content_type_for_key(key) else {
                skipped += 1;
                continue;
            };

            let head = client.head_object().bucket(&bucket).key(key).send().await.unwrap();
            let current = head.content_type().unwrap_or("");
            if current == expected {
                correct += 1;
                continue;
            }

            println!("{key}: {current:?} -> {expected:?}");
            if apply {
                // Copying an object onto itself with REPLACE rewrites its metadata, so the other
                // headers are carried over explicitly.
                client
                    .copy_object()
                    .bucket(&bucket)
                    .copy_source(format!("/{}/{}", bucket, key))
                    .key(key)
                    .metadata_directive(MetadataDirective::Replace)
                    .content_type(expected)
                    .set_cache_control(head.cache_control().map(str::to_string))
                    .set_content_disposition(head.content_disposition().map(str::to_string))
                    .set_content_encoding(head.content_encoding().map(str::to_string))
                    .set_content_language(head.content_language().map(str::to_string))
                    .set_metadata(head.metadata().cloned())
                    .send()
                    .await
                    .unwrap();
            }
            fixed += 1;
        }
    }

    let verb = if apply { "Fixed" } else { "Would fix" };
    println!("Scanned {scanned}. {verb} {fixed}, already correct {correct}, unknown extension {skipped}.");
}

#[derive(Serialize, Deserialize, Clone, Debug)]
struct S3Config {
    pub bucket_name: String,
    pub endpoint_url: String,
    pub public_domain: String,
    pub access_key_id: String,
    pub access_key_secret: String,
}

fn get_client(cfg: S3Config) -> aws_sdk_s3::Client {
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
    aws_sdk_s3::Client::from_conf(config)
}
