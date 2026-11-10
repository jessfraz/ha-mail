#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("mail item not found")]
    NotFound,
    #[error("mailbox binding does not match this database")]
    Binding,
    #[error("sync is already queued")]
    Busy,
    #[error("stored mail data is invalid")]
    Corrupt,
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("stored mail data is invalid")]
    Json(#[from] serde_json::Error),
    #[error("database migration failed")]
    Migration(#[from] sqlx::migrate::MigrateError),
}

impl From<Error> for dropshot::HttpError {
    fn from(error: Error) -> Self {
        use dropshot::ClientErrorStatusCode as Status;
        let (status, code) = match error {
            Error::Invalid(_) => (Status::BAD_REQUEST, "invalid_input"),
            Error::NotFound => (Status::NOT_FOUND, "not_found"),
            Error::Binding => (Status::CONFLICT, "mailbox_binding_mismatch"),
            Error::Busy => (Status::CONFLICT, "sync_already_queued"),
            _ => return Self::for_internal_error("mail storage unavailable".into()),
        };
        Self::for_client_error(Some(code.into()), status, error.to_string())
    }
}
