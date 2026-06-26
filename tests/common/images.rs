use std::borrow::Cow;
use std::collections::HashMap;
use testcontainers_modules::testcontainers::core::wait::HttpWaitStrategy;
use testcontainers_modules::testcontainers::core::WaitFor;
use testcontainers_modules::testcontainers::{CopyToContainer, Image};

#[derive(Debug, Default, Clone)]
pub struct Redis8;

impl Image for Redis8 {
    fn name(&self) -> &str { "redis" }

    fn tag(&self) -> &str { "8.4-alpine" }

    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::message_on_stdout("Ready to accept connections")]
    }
}

#[derive(Debug, Clone)]
pub struct Postgres17 {
    env_vars: HashMap<String, String>,
    copy_to_sources: Vec<CopyToContainer>,
    fsync_enabled: bool,
}

impl Default for Postgres17 {
    fn default() -> Self {
        let mut env_vars = HashMap::new();
        env_vars.insert("POSTGRES_DB".to_owned(), "postgres".to_owned());
        env_vars.insert("POSTGRES_USER".to_owned(), "postgres".to_owned());
        env_vars.insert("POSTGRES_PASSWORD".to_owned(), "postgres".to_owned());

        Self {
            env_vars,
            copy_to_sources: Vec::new(),
            fsync_enabled: false,
        }
    }
}

impl Image for Postgres17 {
    fn name(&self) -> &str { "postgres" }
    fn tag(&self) -> &str { "17.6-alpine" }
    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![
            WaitFor::message_on_stderr("database system is ready to accept connections"),
            WaitFor::message_on_stdout("database system is ready to accept connections"),
        ]
    }

    fn env_vars(&self) -> impl IntoIterator<Item=(impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        &self.env_vars
    }

    fn copy_to_sources(&self) -> impl IntoIterator<Item=&CopyToContainer> {
        &self.copy_to_sources
    }

    fn cmd(&self) -> impl IntoIterator<Item=impl Into<Cow<'_, str>>> {
        if !self.fsync_enabled {
            vec!["-c", "fsync=off"]
        } else {
            vec![]
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Meilisearch1_32;

impl Image for Meilisearch1_32 {
    fn name(&self) -> &str { "getmeili/meilisearch" }
    fn tag(&self) -> &str { "v1.32" }
    fn ready_conditions(&self) -> Vec<WaitFor> {
        vec![WaitFor::http(
            HttpWaitStrategy::new("/health")
                .with_expected_status_code(200_u16)
                .with_body(r#"{ "status": "available" }"#.as_bytes()),
        )]
    }
    fn env_vars(&self) -> impl IntoIterator<Item=(impl Into<Cow<'_, str>>, impl Into<Cow<'_, str>>)> {
        vec![
            ("MEILI_NO_ANALYTICS", "true"),
            ("MEILI_MASTER_KEY", "12345678")
        ]
    }
}