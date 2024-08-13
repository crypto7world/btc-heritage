use btc_heritage::AccountXPubId;
use core::fmt::Debug;
use thiserror::Error;

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("This operation cannot be performed because there is no online wallet component")]
    MissingOnlineWallet,
    #[error(
        "This operation cannot be performed because the online wallet component is not the expected type ({0})"
    )]
    IncorrectOnlineWallet(&'static str),
    #[error("This operation cannot be performed because there is no key provider component")]
    MissingKeyProvider,
    #[error(
        "This operation cannot be performed because the key provider is not the expected type ({0})"
    )]
    IncorrectKeyProvider(&'static str),
    #[error("This operation cannot be performed because there is no heritage provider component")]
    MissingHeritageProvider,
    #[error("A wallet cannot have neither online and offline components")]
    NoComponent,
    #[error("The different parts don't have the same fingerprint")]
    IncoherentFingerprints,
    #[error("The online wallet does not yet have a bound fingerprint")]
    OnlineWalletFingerprintNotPresent,
    #[error("No item named \"{0}\" in the database")]
    InexistantItem(String),
    #[error("An item named \"{0}\" is already in the database")]
    ItemAlreadyExist(String),
    #[error("No wallet named \"{0}\" in the database")]
    InexistantWallet(String),
    #[error("A wallet named \"{0}\" is already in the database")]
    WalletAlreadyExist(String),
    #[error("No heir named \"{0}\" in the database")]
    InexistantHeir(String),
    #[error("An heir named \"{0}\" is already in the database")]
    HeirAlreadyExist(String),
    #[error("The Descriptor {descriptor} is invalid: {error}")]
    InvalidDescriptor { descriptor: String, error: String },
    #[error("{0}")]
    InvalidAddressNetwork(String),
    #[error("Password is missing for LocalKey with password")]
    LocalKeyMissingPassword,
    #[error("The descriptor cannot be transformed in a Ledger wallet policy (reason: {0})")]
    LedgerIncompatibleDescriptor(&'static str),
    #[error("Missing registered Ledger policy (wanted: {0:?})")]
    LedgerMissingRegisteredPolicy(Vec<AccountXPubId>),
    #[error("HeirConfig from Ledger are not supported because we cannot sign Heir transactions at the moment")]
    LedgerHeirUnsupported,
    #[error("It is impossible to extract the wallet Mnemonic from a Ledger device")]
    LedgerBackupMnemonicUnsupported,
    #[error("The account derivation index {0} is too big (max 2^31-1)")]
    AccountDerivationIndexOutOfBound(u32),
    #[error("No wallet found in the service")]
    NoServiceWalletFound,
    #[error("Multiple wallets found in the service")]
    MultipleServiceWalletsFound,
    #[error("No heir found in the service")]
    NoServiceHeirFound,
    #[error("Multiple heirs found in the service")]
    MultipleServiceHeirsFound,
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
impl Error {
    pub fn generic(e: impl core::fmt::Display) -> Self {
        Self::Generic(e.to_string())
    }
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