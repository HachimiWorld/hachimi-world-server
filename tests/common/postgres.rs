//! Postgres for integration tests.
//!
//! With `TEST_DATABASE_URL` set (e.g. `postgres://postgres:postgres@localhost:5432/postgres`), every
//! test gets its own database cloned from a migrated template on that server, so tests share one
//! Postgres instead of starting a container each. Without it, each test starts its own container.

use crate::common::images::Postgres17;
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Connection, PgConnection, PgPool};
use std::str::FromStr;
use std::time::{Duration, Instant};
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::ContainerAsync;
use tracing::info;

/// Keeps the test database alive; call [TestPostgres::cleanup] when the test is done.
pub enum TestPostgres {
    Container(ContainerAsync<Postgres17>),
    Shared { admin: PgConnectOptions, database: String },
}

impl TestPostgres {
    pub async fn start() -> (TestPostgres, PgPool) {
        match std::env::var("TEST_DATABASE_URL") {
            Ok(url) => start_shared(&url).await,
            Err(_) => start_container().await,
        }
    }

    pub async fn cleanup(self, pool: PgPool) {
        pool.close().await;
        match self {
            TestPostgres::Container(instance) => drop(instance),
            TestPostgres::Shared { admin, database } => {
                let mut conn = PgConnection::connect_with(&admin).await.unwrap();
                run(&mut conn, format!(r#"DROP DATABASE IF EXISTS "{database}" WITH (FORCE)"#)).await;
            }
        }
    }
}

async fn start_container() -> (TestPostgres, PgPool) {
    let instance = Postgres17::default().start().await.unwrap();
    let host_ip = instance.get_host().await.unwrap();
    let host_port = instance.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host_ip}:{host_port}/postgres");
    info!("Open test postgres instance at {}", url);
    let pool = PgPool::connect(&url).await.unwrap();
    sqlx::migrate!().run(&pool).await.unwrap();
    (TestPostgres::Container(instance), pool)
}

async fn start_shared(url: &str) -> (TestPostgres, PgPool) {
    let admin = PgConnectOptions::from_str(url).unwrap();
    let mut conn = connect_with_retry(&admin).await;
    let template = ensure_template(&mut conn, &admin).await;

    let database = format!("hachimi_test_{}", uuid::Uuid::new_v4().simple());
    run(&mut conn, format!(r#"CREATE DATABASE "{database}" TEMPLATE "{template}""#)).await;
    conn.close().await.ok();
    info!("Open test database {} on the shared postgres", database);

    // Many tests share the server, which allows 100 connections by default.
    let pool = PgPoolOptions::new().max_connections(5).connect_with(admin.clone().database(&database)).await.unwrap();
    (TestPostgres::Shared { admin, database }, pool)
}

/// Creates the migrated template once per set of migrations. Named after the migrations' checksums,
/// so a long-lived server builds a new one when they change.
async fn ensure_template(conn: &mut PgConnection, admin: &PgConnectOptions) -> String {
    let migrator = sqlx::migrate!();
    let mut hasher = Sha256::new();
    for migration in migrator.iter() {
        hasher.update(migration.version.to_le_bytes());
        hasher.update(&migration.checksum);
    }
    let template = format!("hachimi_template_{}", &hex::encode(hasher.finalize())[..16]);

    // Other test processes start at the same time; only one of them builds the template.
    const LOCK_KEY: i64 = 0x6861_6368_696d_69; // "hachimi"
    sqlx::query("SELECT pg_advisory_lock($1)").bind(LOCK_KEY).execute(&mut *conn).await.unwrap();

    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
        .bind(&template)
        .fetch_one(&mut *conn).await.unwrap();
    if !exists {
        // Build under another name and rename at the end, so a half-built template from a crashed
        // run is never cloned.
        let building = format!("{template}_building");
        run(&mut *conn, format!(r#"DROP DATABASE IF EXISTS "{building}" WITH (FORCE)"#)).await;
        run(&mut *conn, format!(r#"CREATE DATABASE "{building}""#)).await;
        let mut build_conn = PgConnection::connect_with(&admin.clone().database(&building)).await.unwrap();
        migrator.run(&mut build_conn).await.unwrap();
        build_conn.close().await.unwrap();
        run(&mut *conn, format!(r#"ALTER DATABASE "{building}" RENAME TO "{template}""#)).await;
        info!("Created test database template {}", template);
    }

    sqlx::query("SELECT pg_advisory_unlock($1)").bind(LOCK_KEY).execute(&mut *conn).await.unwrap();
    template
}

/// The shared server may still be starting when the first tests run.
async fn connect_with_retry(options: &PgConnectOptions) -> PgConnection {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match PgConnection::connect_with(options).await {
            Ok(conn) => return conn,
            Err(e) if Instant::now() < deadline => {
                info!("Waiting for the shared postgres: {}", e);
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("Cannot connect to TEST_DATABASE_URL: {e}"),
        }
    }
}

/// Runs a statement built from names this module generates, never from outside input.
async fn run(conn: &mut PgConnection, sql: String) {
    sqlx::raw_sql(AssertSqlSafe(sql)).execute(conn).await.unwrap();
}
