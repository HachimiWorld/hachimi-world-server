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

### 1. Prerequisites

1. Launch postgresql, redis and meilisearch. You can check the example in `docker/docker-compose-example.yaml`
2. Install Rust toolchain. You can check the official doc [here](https://www.rust-lang.org/tools/install).
   
### 2. Setup `sqlx-cli`

More docs about [SQLx CLI](https://github.com/launchbadge/sqlx/blob/main/sqlx-cli/README.md#sqlx-cli).

Install `sqlx-cli`:

```shell
cargo instal sqlx-cli
```

Configure the database URL in the env var `DATABASE_URL`, or create `./.env` dot env file:

```
DATABASE_URL=postgres://username:password@host/database
```

Run SQLx migration to initialize the database schemas:

```shell
cargo sqlx mig run
```

### 3. Build and run

Create a config file in `./config.yaml`. You can check the example in `config/config-example.yaml`

Build and run the server:

```sh
cargo run --bin hachimi-world-server
```

## API Compatibility

Released clients keep running long after a deploy, so an API change must not break them. `openapi.json` describes the API and is generated from the routes. After changing a request or response, regenerate it and commit it:

```sh
cargo run --bin gen_openapi
```

A unit test (`openapi_json_is_current`) and CI fail when it is out of date. CI also fails when `oasdiff breaking` finds a change from master's copy that breaks released clients, such as a removed endpoint or response field, a changed type, a response field that became nullable or optional, or a new required request parameter. Only modules routed with `aide::axum::ApiRouter` are in the document so far.

A new variant of a polymorphic response type, such as a new moment attachment type or feed kind, only warns (`response-property-one-of-added`, set in `.oasdiff-severity-levels`): every such type the client decodes falls back to an unknown variant for kinds it doesn't know. A new polymorphic type in a response needs that fallback in the client too, which this check can't see.

To remove an endpoint or a response field, deprecate it first and remove it in a later release:

1. Clients stop using it from some version, say 261107.
2. Mark it deprecated and get that onto master. For an endpoint, route it with `get_with(handler, deprecated)` (`web::routes::deprecated`); for a field, add `#[schemars(extend("deprecated" = true))]` (not Rust's `#[deprecated]`, which warns wherever the server builds the struct). Say in its doc when it can go: `@deprecated 261110: clients stopped calling it in 261107. Remove once min_version >= 261107.`
3. Once `min_version` reaches that version, remove it. Master's copy has it deprecated, so oasdiff reports the removal as info only. Removing something that wasn't deprecated on master fails, even if the same PR deprecates it.

For any other change the check flags that released clients do handle, copy the reported line into `.oasdiff-err-ignore` (as `METHOD /path message`) in the same PR. The line can go once the change is on master.

## Bug Fix / Performance Improvement

You can directly create PR for bug fixes commits. It's better to create an issue before that.

## New Feature

If you want to add new features, please create a discussion before that.

## Security Report

If you found any security issue, please contact us privately.

Thank you for helping us to make Hachimi World better!