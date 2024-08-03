use core::fmt::Debug;

use btc_heritage::AccountXPubId;
use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "This operation cannot be performed by this wallet because it has no Online component"
    )]
    MissingOnlineComponent,
    #[error(
        "This operation cannot be performed by this wallet because the Online component is not the expected type ({0})"
    )]
    IncorrectOnlineComponent(&'static str),
    #[error(
        "This operation cannot be performed by this wallet because it has no Offline component"
    )]
    MissingOfflineComponent,
    #[error(
        "This operation cannot be performed by this wallet because the Offline component is not the expected type ({0})"
    )]
    IncorrectOfflineComponent(&'static str),
    #[error("A wallet cannot have neither online and offline components")]
    NoComponent,
    #[error("The Online and Offline parts of the wallet don't have the same fingerprint")]
    IncoherentFingerprints,
    #[error("No wallet named \"{0}\" in the database")]
    InexistantWallet(String),
    #[error("A wallet named \"{0}\" is already in the database")]
    WalletAlreadyExist(String),
    #[error("The Descriptor {descriptor} is invalid: {error}")]
    InvalidDescriptor { descriptor: String, error: String },
    #[error("Password is missing for LocalKey with password")]
    LocalKeyMissingPassword,
    #[error("The descriptor cannot be transformed in a Ledger wallet policy (reason: {0})")]
    LedgerIncompatibleDescriptor(&'static str),
    #[error("Missing registered Ledger policy (wanted: {0:?})")]
    LedgerMissingRegisteredPolicy(Vec<AccountXPubId>),
    #[error("HeirConfig from Ledger are not supported because we cannot sign Heir transactions at the moment")]
    LedgerHeirUnsupported,
    #[error("It is impossible to extract the wallet Mnemonic from a Ledger device")]
    LedgerGetMnemonicUnsupported,
    #[error("No wallet found in the service")]
    NoServiceWalletFound,
    #[error("The account derivation index {0} is too big (max 2^31-1)")]
    AccountDerivationIndexOutOfBound(u32),
    #[error("Multiple wallet found in the service")]
    MultipleServiceWalletFound,
    #[error("The wallet fingerprint on the service is not the one stored in the local database")]
    IncoherentServiceWalletFingerprint,
    #[error("The wallet fingerprint on the connected Ledger is not the one stored in the local database")]
    IncoherentLedgerWalletFingerprint,
    #[error("The retrieved wallet fingerprint is not the one stored in the local database. Wrong password.")]
    IncoherentLocalKeyFingerprint,
    #[error("The key {0} is already in the database")]
    KeyAlreadyExists(String),
    #[error("Heritage error: {source}")]
    HeritageError {
        #[from]
        source: btc_heritage::errors::Error,
    },
    #[error("Heritage API client error: {source}")]
    SendRequestError {
        #[from]
        source: heritage_api_client::Error,
    },
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Ledger client error: {0}")]
    LedgerClientError(String),
    #[error("Generic error: {0}")]
    Generic(String),
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Generic(format!("{value}"))
    }
}

impl<T: Debug> From<ledger_bitcoin_client::error::BitcoinClientError<T>> for Error {
    fn from(value: ledger_bitcoin_client::error::BitcoinClientError<T>) -> Self {
        Self::LedgerClientError(format!("{value:?}"))
    }
}

impl From<redb::Error> for Error {
    fn from(value: redb::Error) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
impl From<redb::DatabaseError> for Error {
    fn from(value: redb::DatabaseError) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
impl From<redb::TableError> for Error {
    fn from(value: redb::TableError) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
impl From<redb::TransactionError> for Error {
    fn from(value: redb::TransactionError) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
impl From<redb::CommitError> for Error {
    fn from(value: redb::CommitError) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
impl From<redb::StorageError> for Error {
    fn from(value: redb::StorageError) -> Self {
        Self::DatabaseError(format!("{value}"))
    }
}
