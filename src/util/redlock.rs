use tracing::{debug, error, info};
use redis::aio::ConnectionManager;
use redis::{AsyncCommands, ExistenceCheck, SetExpiry, SetOptions, Value};
use std::sync::{mpsc, Arc};
use std::thread;
use std::thread::JoinHandle;
use chrono::Duration;
use tracing::log::log;

#[derive(Clone)]
pub struct RedLock {
    inner: Arc<RedLockInner>
}

struct RedLockInner {
    unlock_tx: mpsc::Sender<LockInfo>,
    redis_conn: ConnectionManager,
    listen_thread: JoinHandle<()>
}

impl RedLock {
    pub fn new(redis_conn: ConnectionManager) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<LockInfo>();

        let handle = thread::spawn({
            let mut redis_conn = redis_conn.clone();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;

            move || {
                for lock_info in rx {
                    match &rt.block_on(async {
                        unlock(&mut redis_conn, &lock_info).await
                    }) {
                        Ok(_) => {
                            debug!("unlock the resource: {}", lock_info.res_name);
                        }
                        Err(_) => {
                            error!("Failed to unlock the resource: {}", lock_info.res_name);
                        }
                    };
                }
            }
        });

        Ok(Self {
            inner: Arc::new(RedLockInner {
                unlock_tx: tx,
                redis_conn,
                listen_thread: handle
            })
        })
    }

    /// Spin lock until the lock is acquired
    pub async fn lock(&self, res_name: &str) -> anyhow::Result<RedLockGuard> {
        loop {
            match self.try_lock(res_name).await? {
                Some(guard) => return Ok(guard),
                None => {
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Spin lock until the lock is acquired or timeout
    pub async fn lock_with_timeout(&self, res_name: &str, timeout: Duration) -> anyhow::Result<Option<RedLockGuard>> {
        let start = std::time::Instant::now();
        loop {
            match self.try_lock(res_name).await? {
                Some(guard) => return Ok(Some(guard)),
                None => {
                    if start.elapsed().as_secs() > timeout.num_seconds() as u64 {
                        return Ok(None);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Try to acquire the lock
    pub async fn try_lock(&self, res_name: &str) -> anyhow::Result<Option<RedLockGuard>> {
        info!("try to acquire the lock: {}", res_name);
        let mut conn = self.inner.redis_conn.clone();
        let lock_info = LockInfo::new(res_name);
        let opt = SetOptions::default()
            .conditional_set(ExistenceCheck::NX)
            .with_expiration(SetExpiry::PX(30000));
        let result: Value = conn.set_options(res_name, &lock_info.sign, opt).await?;
        match result {
            Value::Okay => {
                let guard = RedLockGuard::new(self.inner.unlock_tx.clone(), lock_info);
                Ok(Some(guard))
            }
            Value::Nil => Ok(None),
            _ => Ok(None)
        }
    }
}

async fn unlock(conn: &mut ConnectionManager, lock: &LockInfo) -> anyhow::Result<()> {
    let sign: Option<u32> = conn.get(&lock.res_name).await?;
    if sign == Some(lock.sign) {
        let _: () = conn.del(&lock.res_name).await?;
    }
    Ok(())
}

pub struct RedLockGuard {
    unlock_tx: mpsc::Sender<LockInfo>,
    lock_info: LockInfo
}

/// Auto unlock the lock when the guard is dropped (async), auto-unlock is not guaranteed
impl RedLockGuard {
    fn new(unlock_tx: mpsc::Sender<LockInfo>, lock_info: LockInfo) -> Self {
        Self {
            unlock_tx,
            lock_info
        }
    }
}

impl Drop for RedLockGuard {
    fn drop(&mut self) {
        self.unlock_tx.send(self.lock_info.clone()).unwrap();
    }
}

#[derive(Debug, Clone)]
struct LockInfo {
    res_name: String,
    sign: u32
}

impl LockInfo {
    fn new(res_name: &str) -> Self {
        Self {
            res_name: res_name.to_string(),
            sign: rand::random::<u32>()
        }
    }
}