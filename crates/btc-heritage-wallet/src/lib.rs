mod database;
pub mod errors;
mod heir;
mod heir_wallet;
mod psbt_summary;
mod traits;
mod wallet;

pub mod heritage_provider;
pub mod key_provider;
pub mod online_wallet;

pub use btc_heritage;
pub mod ledger {
    pub use ledger_bitcoin_client::{wallet::Version, WalletPolicy, WalletPubKey};
}

pub use heritage_provider::{AnyHeritageProvider, Heritage};
pub use key_provider::{
    ledger_hww::{ledger_client, policy::LedgerPolicy, LedgerClient, LedgerKey},
    local_key::LocalKey,
    AnyKeyProvider, HeirConfigType,
};
pub use online_wallet::AnyOnlineWallet;

pub use heir::Heir;
pub use heir_wallet::HeirWallet;
pub use wallet::Wallet;

pub use bip39::{Language, Mnemonic};
pub use btc_heritage::bitcoin;
pub use btc_heritage::miniscript;
pub use database::{Database, DatabaseItem, DatabaseSingleItem};
pub use heritage_service_api_client;
pub use psbt_summary::PsbtSummary;
pub use traits::*;
