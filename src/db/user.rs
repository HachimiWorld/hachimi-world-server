use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgExecutor, PgPool, Pool, Postgres, Result};
use crate::db::CrudDao;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub password_hash: String,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub gender: Option<i32>,
    pub is_banned: bool,
    pub last_login_time: Option<DateTime<Utc>>,
    pub create_time: DateTime<Utc>,
    pub update_time: DateTime<Utc>,
}

pub struct UserDao;

pub trait IUserDao<'e, E>: CrudDao<'e, E>
where E: PgExecutor<'e> {
    async fn get_by_email(executor: E, email: &str) -> Result<Option<User>>;
    async fn get_by_username(executor: E, username: &str) -> Result<Option<User>>;
    async fn get_by_ids(executor: E, ids: &Vec<i64>) -> Result<Vec<User>>;
}

impl <'e, E> CrudDao<'e, E> for UserDao
where E: PgExecutor<'e> {
    type Entity = User;

    async fn list(executor: E) -> Result<Vec<User>> {
        sqlx::query_as!(User, "SELECT * FROM users")
            .fetch_all(executor)
            .await
    }

    async fn page(executor: E, page: i64, size: i64) -> Result<Vec<User>> {
        Ok(sqlx::query_as!(User, "SELECT * FROM users LIMIT $1 OFFSET $2", size, (page - 1) * size)
            .fetch_all(executor)
            .await?)
    }

    async fn get_by_id(executor: E, id: i64) -> Result<Option<User>> {
        Ok(sqlx::query_as!(User, "SELECT * FROM users WHERE id = $1", id)
            .fetch_optional(executor)
            .await?)
    }

    async fn insert(executor: E, value: &User) -> Result<i64> {
        let result = sqlx::query!(
            "INSERT INTO users(username, email, password_hash, avatar_url, bio, gender, is_banned, last_login_time, create_time, update_time) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING id",
            value.username,
            value.email,
            value.password_hash,
            value.avatar_url,
            value.bio,
            value.gender,
            value.is_banned,
            value.last_login_time,
            value.create_time,
            value.update_time,
        ).fetch_one(executor).await?;

        Ok(result.id)
    }

    async fn update_by_id(executor: E, value: &User) -> Result<()> {
        sqlx::query!(
            "UPDATE users SET username = $1, email = $2, password_hash = $3, avatar_url = $4, bio = $5, gender = $6, is_banned = $7, last_login_time = $8, create_time = $9, update_time = $10 WHERE id = $11",
            value.username,
            value.email,
            value.password_hash,
            value.avatar_url,
            value.bio,
            value.gender,
            value.is_banned,
            value.last_login_time,
            value.create_time,
            value.update_time,
            value.id
        ).execute(executor).await?;
        Ok(())
    }

    async fn delete_by_id(executor: E, id: i64) -> Result<()> {
        sqlx::query!("DELETE FROM users WHERE id = $1", id)
            .execute(executor)
            .await?;
        Ok(())
    }
}

impl <'e, E> IUserDao<'e, E> for UserDao 
where E: PgExecutor<'e> {
    async fn get_by_email(executor: E, email: &str) -> Result<Option<User>> {
        Ok(sqlx::query_as!(User, "SELECT * FROM users WHERE email = $1", email)
            .fetch_optional(executor)
            .await?)
    }

    async fn get_by_username(executor: E, username: &str) -> Result<Option<User>> {
        sqlx::query_as!(User, "SELECT * FROM users WHERE username = $1", username)
            .fetch_optional(executor)
            .await
    }

    async fn get_by_ids(executor: E, ids: &Vec<i64>) -> Result<Vec<User>> {
        sqlx::query_as!(User, "SELECT * FROM users WHERE id = ANY($1)", ids)
            .fetch_all(executor)
            .await
    }
}