# Contributing Guide

This project uses the following major technologies:
- Rust - Programming language
  - `tokio` - Async runtime
  - `axum` - HTTP server framework based on tokio
  - `sqlx` - Database access
  - `serde` - Serialization/Deserialization
- PostgreSQL - Database
- Redis/Valkey - Cache
- Meilisearch - Full text search engine
- Docker

If you have any question, feel free to create issues/discussions, or join our discord.

## Build Guide

1. Launch postgresql, redis and meilisearch. You can check the example in `docer/docker-compose-example.yaml`
2. Create a config file in `./config.yaml`. You can check the example in `config/config-example.yaml`
3. Build and run the server. The table schemas will be created automatically.

```sh
cargo run --bin hachimi-world-server
```

## Bug Fix / Performance Improvement

You can directly create PR for bug fixes commits. It's better to create an issue before that.

## New Feature

If you want to add new features, please create a discussion before that.

## Security Report

If you found any security issue, please contact us privately.

Thank you for helping us to make Hachimi World better!