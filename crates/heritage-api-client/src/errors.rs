use core::fmt::Debug;

use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("The authentication process expired")]
    AuthenticationProcessExpired,
    #[error("The client is not authenticated to the Heritage service API.")]
    Unauthenticated,
    #[error("The client received an unexpected response that could not be parsed: {source}")]
    MalformedJsonResponse {
        #[from]
        source: serde_json::Error,
    },
    #[error("Could not send the request: {source}")]
    SendRequestError {
        #[from]
        source: reqwest::Error,
    },
    #[error("Cannot retrieve the response body string")]
    UnretrievableBodyResponse,
    #[error("Could not read the tokens from the cache: {0}")]
    TokenCacheReadError(String),
    #[error("Could not write the tokens in the cache: {0}")]
    TokenCacheWriteError(String),
    #[error("Heritage API responded with error {code}: {message}")]
    ApiErrorResponse { code: u16, message: String },
    #[error("Generic error: {0}")]
    Generic(String),
}
