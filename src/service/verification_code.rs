use crate::db::user::UserId;
use rand::RngExt;
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, ExistenceCheck, SetExpiry, SetOptions};

/// Redis keys of one kind of code. A code stored under one set of keys can't be used with another,
/// so a code for registering can't change an email.
pub struct CodeKeys {
    code: String,
    retries: String,
    limited: String,
}

impl CodeKeys {
    /// Codes sent to `email` to register, log in or reset the password.
    pub fn email(email: &str) -> Self {
        CodeKeys {
            code: get_verify_code_key(email),
            retries: get_verify_code_retries_key(email),
            limited: get_verify_code_limited_key(email),
        }
    }

    /// Codes sent to the current address of `uid`, to change it.
    /// @since 261105
    pub fn change_email_old(uid: UserId) -> Self {
        CodeKeys {
            code: format!("email_change_code:old:{uid}"),
            retries: format!("email_change_code:old:retries:{uid}"),
            limited: format!("email_change_code:old:limited:{uid}"),
        }
    }

    /// Codes sent to `new_email`, the address `uid` wants to change to. Sending is limited per
    /// user, not per address, so one user can't send to many addresses at once.
    /// @since 261105
    pub fn change_email_new(uid: UserId, new_email: &str) -> Self {
        CodeKeys {
            code: format!("email_change_code:new:{uid}:{new_email}"),
            retries: format!("email_change_code:new:retries:{uid}:{new_email}"),
            limited: format!("email_change_code:new:limited:{uid}"),
        }
    }
}

/// Automatically:
/// - increase the retry counter when error
/// - del the code when succeed or max retries
pub async fn verify_code(
    conn: &mut ConnectionManager,
    email: &str,
    code: &str,
) -> anyhow::Result<bool> {
    verify_code_with(conn, &CodeKeys::email(email), code).await
}

/// [verify_code] for codes stored under `keys`.
pub async fn verify_code_with(
    conn: &mut ConnectionManager,
    keys: &CodeKeys,
    code: &str,
) -> anyhow::Result<bool> {
    let value: Option<String> = conn.get(&keys.code).await?;
    if let Some(v) = value && v == code {
        let _: () = conn.del(&keys.code).await?;
        Ok(true)
    } else {
        let retires: i32 = conn.incr(&keys.retries, 1).await?;
        if retires > 3 {
            // Invalidate code
            let _: () = conn.del(&keys.code).await?;
        }
        Ok(false)
    }
}

/// Returns false means limited
pub async fn set_limit_nx(conn: &mut ConnectionManager, email: &str) -> anyhow::Result<bool> {
    set_limit_nx_with(conn, &CodeKeys::email(email)).await
}

/// [set_limit_nx] for codes stored under `keys`.
pub async fn set_limit_nx_with(conn: &mut ConnectionManager, keys: &CodeKeys) -> anyhow::Result<bool> {
    let limit_absent: bool = conn
        .set_options(
            &keys.limited,
            0,
            SetOptions::default()
                .conditional_set(ExistenceCheck::NX)
                .with_expiration(SetExpiry::EX(60)),
        )
        .await?;
    Ok(limit_absent)
}

pub async fn set_code(conn: &mut ConnectionManager, email: &str, code: &str) -> anyhow::Result<()> {
    set_code_with(conn, &CodeKeys::email(email), code).await
}

/// [set_code] for codes stored under `keys`.
pub async fn set_code_with(conn: &mut ConnectionManager, keys: &CodeKeys, code: &str) -> anyhow::Result<()> {
    let _: () = conn.set_ex(&keys.code, code, 300).await?;

    // Reset retries
    let _: () = conn.set_ex(&keys.retries, 0, 300).await?;
    Ok(())
}

pub fn generate_verify_code() -> String {
    format!("{:06}", rand::rng().random_range(0..1000000))
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
            assert_eq!(6, code.len())
        }
    }
}
