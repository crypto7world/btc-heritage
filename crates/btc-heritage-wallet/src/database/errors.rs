use btc_heritage::bdk_types;
use core::fmt::Debug;
use thiserror::Error;

use super::DatabaseTransactionOperation;

pub type Result<T> = core::result::Result<T, DbError>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("The table {0} does not exist in the database")]
    TableDoesNotExists(String),
    #[error("The table {0} already exists in the database")]
    TableAlreadyExists(String),
    #[error("The key {0} does not exist in the database")]
    KeyDoesNotExists(String),
    #[error("The key {0} is already in the database")]
    KeyAlreadyExists(String),
    #[error("The database transaction could not be completed, operation #{idx} ({op:?}) failed")]
    TransactionFailed {
        idx: usize,
        op: DatabaseTransactionOperation,
        reason: String,
    },
    #[error("The key {0} did not have the expected value")]
    CompareAndSwapError(String),
    #[error("Could not serialize for key {key}: {error}")]
    SerDeError { key: String, error: String },
    #[error("Prefix must not be empty")]
    EmptyPrefix,
    #[error("RedbError: {0}")]
    RedbError(redb::Error),
    #[error("Generic DbError: {0}")]
    Generic(String),
}
impl DbError {
    pub fn generic(e: impl core::fmt::Display) -> Self {
        Self::Generic(e.to_string())
    }
    pub fn serde(k: impl Into<String>, e: serde_json::Error) -> Self {
        Self::SerDeError {
            key: k.into(),
            error: e.to_string(),
        }
    }
}

impl From<redb::DatabaseError> for DbError {
    fn from(value: redb::DatabaseError) -> Self {
        Self::RedbError(value.into())
    }
}
impl From<redb::TableError> for DbError {
    fn from(value: redb::TableError) -> Self {
        Self::RedbError(value.into())
    }
}
impl From<redb::TransactionError> for DbError {
    fn from(value: redb::TransactionError) -> Self {
        Self::RedbError(value.into())
    }
}
impl From<redb::CommitError> for DbError {
    fn from(value: redb::CommitError) -> Self {
        Self::RedbError(value.into())
    }
}
impl From<redb::StorageError> for DbError {
    fn from(value: redb::StorageError) -> Self {
        Self::RedbError(value.into())
    }
}

impl From<DbError> for bdk_types::Error {
    fn from(value: DbError) -> Self {
        log::error!("{value:?}");
        bdk_types::Error::Generic(value.to_string())
    }
}

impl From<DbError> for btc_heritage::errors::DatabaseError {
    fn from(value: DbError) -> Self {
        log::error!("{value:?}");
        btc_heritage::errors::DatabaseError::Generic(value.to_string())
    }
}
