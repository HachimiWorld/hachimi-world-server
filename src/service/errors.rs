pub type ServiceResult<T, K> = Result<T, ServiceError<K>>;

pub enum ServiceError<K> {
    BusinessError(K),
    Other(anyhow::Error),
}

impl<E, F> From<E> for ServiceError<F>
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self::Other(err.into())
    }
}
