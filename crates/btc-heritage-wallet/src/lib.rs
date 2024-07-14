mod database;
pub mod errors;
mod service_client;
mod utils;
mod wallet;
mod wallet_offline;
mod wallet_online;

pub use btc_heritage::{AccountXPub, DescriptorsBackup, HeritageConfig};
pub use wallet::Wallet;
pub use wallet_offline::{
    ledger_hww::{policy::LedgerPolicy, LedgerKey},
    AnyWalletOffline, WalletOffline,
};
pub use wallet_online::{AnyWalletOnline, ServiceBinding, WalletOnline};

pub use btc_heritage::bitcoin;
pub use btc_heritage::miniscript;
pub use database::Database;
pub use service_client::{
    AccountXPubWithStatus, HeritageServiceClient, NewTx, NewTxRecipient, Tokens,
};
