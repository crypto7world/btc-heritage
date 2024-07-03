use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "This operation cannot be performed by this wallet because it has no Online component"
    )]
    MissingOnlineComponent,
    #[error(
        "This operation cannot be performed by this wallet because it has no Offline component"
    )]
    MissingOfflineComponent,
    #[error("A wallet cannot have neither online and offline components")]
    NoComponent,
    #[error("The Online and Offline parts of the wallet don't have the same fingerprint")]
    IncoherentFingerprints,
    #[error("The authentication of the CLI expired")]
    AuthenticationProcessExpired,
    #[error("The CLI is not authenticated to the Heritage service. Login first.")]
    Unauthenticated,
    #[error("No wallet named {0} in the database")]
    InexistantWallet(String),
    #[error("The key {0} is already in the database")]
    KeyAlreadyExists(String),
    #[error("Reqwest error: {source}")]
    ReqwestError {
        #[from]
        source: reqwest::Error,
    },
    #[error("Database error: {source:#}")]
    DatabaseError {
        #[from]
        source: sled::Error,
    },
    #[error("Generic error: {0}")]
    Generic(String),
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Generic(format!("{value}"))
    }
}
impl From<std::str::Utf8Error> for Error {
    fn from(value: std::str::Utf8Error) -> Self {
        Self::Generic(format!("{value}"))
    }
}
impl From<std::string::FromUtf8Error> for Error {
    fn from(value: std::string::FromUtf8Error) -> Self {
        Self::Generic(format!("{value}"))
    }
}
