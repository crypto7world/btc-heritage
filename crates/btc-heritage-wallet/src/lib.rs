mod database;
pub mod errors;
mod utils;
mod wallet;
mod wallet_offline;
mod wallet_online;

pub use btc_heritage::{AccountXPub, DescriptorsBackup, HeritageConfig};
pub use wallet::Wallet;
pub use wallet_offline::{
    ledger_hww::{policy::LedgerPolicy, LedgerKey},
    local_key::LocalKey,
    AnyWalletOffline, HeirConfigType, WalletOffline,
};
pub use wallet_online::{AnyWalletOnline, ServiceBinding, WalletOnline};

pub use bip39::{Language, Mnemonic};
pub use btc_heritage::bitcoin;
pub use btc_heritage::miniscript;
pub use database::Database;
pub use heritage_api_client;
