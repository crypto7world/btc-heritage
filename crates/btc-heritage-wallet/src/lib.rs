mod database;
pub mod errors;
mod heir;
mod psbt_summary;
mod traits;
mod utils;
mod wallet;

pub mod heritage_provider;
pub mod key_provider;
pub mod wallet_online;

pub use btc_heritage;

pub use heritage_provider::{AnyHeritageProvider, Heritage};
pub use key_provider::{
    ledger_hww::{policy::LedgerPolicy, LedgerKey},
    local_key::LocalKey,
    AnyKeyProvider, HeirConfigType,
};
pub use wallet_online::AnyWalletOnline;

pub use heir::{Heir, HeirWallet};
pub use wallet::Wallet;

pub use bip39::{Language, Mnemonic};
pub use btc_heritage::bitcoin;
pub use btc_heritage::miniscript;
pub use database::{Database, DatabaseItem};
pub use heritage_api_client;
pub use psbt_summary::PsbtSummary;
pub use traits::*;
