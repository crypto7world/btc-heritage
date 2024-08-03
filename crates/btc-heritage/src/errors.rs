use thiserror::Error;

use crate::{
    account_xpub::AccountXPubId,
    bitcoin::{psbt::Psbt, Network},
    heritage_wallet::SubwalletConfigId,
};

pub type Result<T> = core::result::Result<T, Error>;

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
    #[error("HeritageWallet was never synchronized")]
    UnsyncedWallet,
    #[error("HeritageWallet does not have any unused AccountXPub")]
    MissingUnusedAccountXPub,
    #[error("An AccountXPub have a different Fingerprint than the Heritage wallet")]
    InvalidAccountXPub,
    #[error("HeritageConfig is not the expected version: {0}")]
    InvalidHeritageConfigVersion(&'static str),
    #[error("Invalid DescriptorPublicKey for AccountXPub: {0}")]
    InvalidDescriptorPublicKey(&'static str),
    #[error("Database error: {0}")]
    DatabaseError(#[from] DatabaseError),
    #[error("Policy extract error while constructing the PSBT: {0}")]
    FailToExtractPolicy(bdk::descriptor::policy::PolicyError),
    #[error("PSBT creation error: {0}")]
    PsbtCreationError(String),
    #[error("Error while interacting with the Blockchain provider: {0}")]
    BlockchainProviderError(String),
    #[error("Error during subwallet synchronization: {0}")]
    SyncError(String),
    #[error("Unknown error: {0}")]
    Unknown(String),
}

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
