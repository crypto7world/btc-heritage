use crate::{
    errors::{Error, Result},
    BoundFingerprint, Broadcaster,
};
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Txid},
    heritage_config::HeritageConfig,
    heritage_wallet::{TransactionSummary, WalletAddress},
    AccountXPub, HeritageWalletBackup, HeritageWalletBalance, PartiallySignedTransaction,
};

mod local_heritage_wallet;
mod service;
use heritage_api_client::{AccountXPubWithStatus, NewTx};
use local_heritage_wallet::LocalHeritageWallet;
use serde::{Deserialize, Serialize};
pub use service::ServiceBinding;

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletInfo {
    pub fingerprint: Option<Fingerprint>,
    pub balance: HeritageWalletBalance,
    pub last_sync_ts: u64,
}

/// This trait regroup the functions of an Heritage wallet that does not need
/// access to the private keys and can be safely operated in an online environment.
pub trait WalletOnline: Broadcaster + BoundFingerprint {
    fn backup_descriptors(&self) -> Result<HeritageWalletBackup>;
    fn get_address(&self) -> Result<String>;
    fn list_addresses(&self) -> Result<Vec<WalletAddress>>;
    fn list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>>;
    fn feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()>;
    fn list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>;
    fn set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig>;
    fn sync(&mut self) -> Result<()>;
    fn get_wallet_info(&self) -> Result<WalletInfo>;
    fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)>;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AnyWalletOnline {
    None,
    Service(ServiceBinding),
    Local(LocalHeritageWallet),
}

impl AnyWalletOnline {
    pub fn is_none(&self) -> bool {
        match self {
            AnyWalletOnline::None => true,
            _ => false,
        }
    }
}

macro_rules! impl_wallet_online_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            impl_wallet_online_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            impl_wallet_online_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($self:ident $fn_name:ident($($a:ident : $t:ty),*)) => {
            match $self {
                AnyWalletOnline::None => Err(Error::MissingOnlineWallet),
                AnyWalletOnline::Service(sb) => sb.$fn_name($($a),*),
                AnyWalletOnline::Local(lhe) => lhe.$fn_name($($a),*),
            }
    };
}

impl WalletOnline for AnyWalletOnline {
    impl_wallet_online_fn!(backup_descriptors(&self) -> Result<HeritageWalletBackup>);
    impl_wallet_online_fn!(get_address(&self) -> Result<String>);
    impl_wallet_online_fn!(list_addresses(&self) -> Result<Vec<WalletAddress>>);
    impl_wallet_online_fn!(list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>>);
    impl_wallet_online_fn!(feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()>);
    impl_wallet_online_fn!(list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>);
    impl_wallet_online_fn!(set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig>);
    impl_wallet_online_fn!(sync(&mut self) -> Result<()>);
    impl_wallet_online_fn!(get_wallet_info(&self) -> Result<WalletInfo>);
    impl_wallet_online_fn!(create_psbt(&self, spending_config: NewTx) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}
impl Broadcaster for AnyWalletOnline {
    impl_wallet_online_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
impl BoundFingerprint for AnyWalletOnline {
    impl_wallet_online_fn!(fingerprint(&self) -> Result<Fingerprint>);
}
