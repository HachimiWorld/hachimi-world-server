use rand::Rng;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, ExistenceCheck, SetExpiry, SetOptions};

/// Automatically:
/// - increase the retry counter when error
/// - del the code when succeed or max retries
pub async fn verify_code(
    conn: &mut ConnectionManager,
    email: &str,
    code: &str,
) -> anyhow::Result<bool> {
    let key = get_verify_code_key(email);
    let value: Option<String> = conn.get(&key).await?;
    if let Some(v) = value && v == code {
        let _: () = conn.del(key).await?;
        Ok(true)
    } else {
        let retires: i32 = conn.incr(get_verify_code_retries_key(email), 1).await?;
        if retires > 3 {
            // Invalidate code
            let _: () = conn.del(key).await?;
        }
        Ok(false)
    }
}

/// Returns false means limited
pub async fn set_limit_nx(conn: &mut ConnectionManager, email: &str) -> anyhow::Result<bool> {
    let limit_absent: bool = conn
        .set_options(
            get_verify_code_limited_key(email),
            0,
            SetOptions::default()
                .conditional_set(ExistenceCheck::NX)
                .with_expiration(SetExpiry::EX(60)),
        )
        .await?;
    Ok(limit_absent)
}

pub async fn set_code(conn: &mut ConnectionManager, email: &str, code: &str) -> anyhow::Result<()> {
    let key = get_verify_code_key(email);
    let _: () = conn.set_ex(key, code, 300).await?;

    // Reset retries
    let retires_key = get_verify_code_retries_key(email);
    let _: () = conn.set(retires_key, 0).await?;
    Ok(())
}

pub fn generate_verify_code() -> String {
    format!("{:08}", rand::rng().random_range(0..100000000))
}

pub fn get_verify_code_key(email: &str) -> String {
    format!("email_code:{}", email)
}

pub fn get_verify_code_limited_key(email: &str) -> String {
    format!("email_code:limited:{}", email)
}

pub fn get_verify_code_retries_key(email: &str) -> String {
    format!("email_code:retries:{}", email)
}

#[cfg(test)]
mod test {
    use crate::service::verification_code::generate_verify_code;

    #[test]
    fn test_gen_verify_code() {
        for _ in 0..100 {
            let code = generate_verify_code();
            assert_eq!(8, code.len())
        }
    }
}
