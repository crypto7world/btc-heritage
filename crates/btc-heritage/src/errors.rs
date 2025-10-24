use thiserror::Error;

use crate::{
    account_xpub::AccountXPubId,
    bitcoin::{psbt::Psbt, Network},
    heritage_wallet::SubwalletConfigId,
    BlockInclusionObjective,
};

/// Convenience type alias for Results using this crate's Error type
pub type Result<T> = core::result::Result<T, Error>;

/// Main error type for the heritage wallet library
///
/// This enum encompasses all possible errors that can occur during heritage wallet
/// operations, from address validation to PSBT creation and blockchain interaction.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0} is not a valid wallet address")]
    InvalidWalletAddressString(String),
    #[error("{0} is not a valid Bitcoin address for the expected network ({1})")]
    InvalidAddressString(String, Network),
    #[error("Psbt is not finalizable: {}", serde_json::json!(.0))]
    UnfinalizablePsbt(Psbt),
    #[error("Trying to call SubwalletConfig::mark_subwallet_firstuse on an already used SubwalletConfig")]
    SubwalletConfigAlreadyMarkedUsed,
    #[error("Trying to set a new HeritageConfig that was already used in this HeritageWallet")]
    HeritageConfigAlreadyUsed,
    #[error("Heirs can only spend by draining the wallet")]
    InvalidSpendingConfigForHeir,
    #[error("HeritageWallet does not have a Current SubwalletConfig")]
    MissingCurrentSubwalletConfig,
    #[error("The current HeritageConfig is expired and cannot be used to generate new addresses")]
    CannotGenerateNewAddress,
    #[error("HeritageWallet was never synchronized")]
    UnsyncedWallet,
    #[error("HeritageWallet does not have any unused AccountXPub")]
    MissingUnusedAccountXPub,
    #[error("An AccountXPub have a different Fingerprint than the Heritage wallet")]
    InvalidAccountXPub,
    #[error("HeritageConfig is not the expected version: {0}")]
    InvalidHeritageConfigVersion(&'static str),
    #[error("{0} cannot be parsed into an HeritageConfigVersion")]
    InvalidHeritageConfigString(String),
    #[error("Invalid DescriptorPublicKey for AccountXPub: {0}")]
    InvalidDescriptorPublicKey(&'static str),
    #[error("Invalid backup: {0}")]
    InvalidBackup(&'static str),
    #[error("Invalid script fragments to recompose {0} Heritage Config")]
    InvalidScriptFragments(&'static str),
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Policy extract error while constructing the PSBT: {0}")]
    FailToExtractPolicy(bdk::descriptor::policy::PolicyError),
    #[error("Failed to reset the address index: {0}")]
    FailedToResetAddressIndex(String),
    #[error("PSBT creation error: {0}")]
    PsbtCreationError(String),
    #[error("UTXOs were requested to be both included and excluded: {0:?}")]
    InvalidUtxoSelectionIncludeExclude(Vec<crate::bitcoin::OutPoint>),
    #[error("Some UTXOs were requested to include that do not exist: {0:?}")]
    UnknownUtxoSelectionInclude(Vec<crate::bitcoin::OutPoint>),
    #[error("Error while interacting with the Blockchain provider: {0}")]
    BlockchainProviderError(String),
    #[error("Error during subwallet synchronization: {0}")]
    SyncError(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}

/// Database-specific errors for heritage wallet operations
///
/// These errors occur during transactional database operations.
#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("SubwalletConfig already present in DB at index={0:?}")]
    SubwalletConfigAlreadyExist(SubwalletConfigId),
    #[error("The Current SubwalletConfig in the database does not have the expected value")]
    UnexpectedCurrentSubwalletConfig,
    #[error("AccountXPub is no longer in the database: {0}")]
    AccountXPubInexistant(AccountXPubId),
    #[error("Generic database error: {0}")]
    Generic(String),
}

/// Errors that can occur when parsing block inclusion objective values
///
/// Block inclusion objectives must be valid integers within the acceptable
/// range for Bitcoin Core's fee estimation (1-1008 blocks).
#[derive(Debug, Error)]
pub enum ParseBlockInclusionObjectiveError {
    #[error("Value could not be parsed as an 16-bits unsigned integer")]
    /// The provided value could not be parsed as a valid integer
    InvalidInt,
    #[error("Value is less than {}", BlockInclusionObjective::MIN)]
    /// The provided value is below the minimum allowed (1 block)
    ValueTooLow,
    #[error("Value is more than {}", BlockInclusionObjective::MAX)]
    /// The provided value is above the maximum allowed (1008 blocks)
    ValueTooHigh,
}
