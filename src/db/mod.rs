pub mod refresh_token;
pub mod user;
pub mod song;
pub mod song_tag;

pub trait CrudDao {
    type Entity;
    async fn list(&self) -> sqlx::Result<Vec<Self::Entity>>;

    async fn page(&self, page: i64, size: i64) -> sqlx::Result<Vec<Self::Entity>>;

    async fn get_by_id(&self, id: i64) -> sqlx::Result<Option<Self::Entity>>;
    async fn update_by_id(&self, value: &Self::Entity) -> sqlx::Result<()>;
    async fn insert(&self, value: &Self::Entity) -> sqlx::Result<i64>;
    async fn delete_by_id(&self, id: i64) -> sqlx::Result<()>;
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
