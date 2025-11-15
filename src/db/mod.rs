use sqlx::PgExecutor;

pub mod refresh_token;
pub mod user;
pub mod song;
pub mod song_tag;
pub mod playlist;
pub mod song_publishing_review;
pub mod version;
pub mod creator;

pub trait CrudDao<'e, E>
where E: PgExecutor<'e> {
    type Entity;
    async fn list(executor: E) -> sqlx::Result<Vec<Self::Entity>>;
    async fn page(executor: E, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>>;

    async fn get_by_id(executor: E, id: i64) -> sqlx::Result<Option<Self::Entity>>;
    async fn update_by_id(executor: E, value: &Self::Entity) -> sqlx::Result<()>;
    async fn insert(executor: E, value: &Self::Entity) -> sqlx::Result<i64>;
    async fn delete_by_id(executor: E, id: i64) -> sqlx::Result<()>;
}

#[cfg(test)]
mod test {
    use sqlx::PgPool;

    pub async fn get_test_pool() -> PgPool {
        dotenv::dotenv().ok();
        let url = std::env::var("TEST_DATABASE_URL")
            .expect("TEST_DATABASE_URL must be set for unit test");
        PgPool::connect(&url)
            .await
            .expect("Failed to connect to test database")
    }
}
