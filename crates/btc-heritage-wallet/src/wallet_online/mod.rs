use crate::errors::{Error, Result};
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Network},
    heritage_config::HeritageConfig,
    heritage_wallet::{DescriptorsBackup, TransactionSummary},
    AccountXPub, HeritageWalletBalance, PartiallySignedTransaction, SpendingConfig,
};

mod local_heritage_wallet;
mod service;
use local_heritage_wallet::LocalHeritageWallet;
use serde::{Deserialize, Serialize};
pub use service::ServiceBinding;

/// This trait regroup the functions of an Heritage wallet that does not need
/// access to the private keys and can be safely operated in an online environment.
pub trait WalletOnline {
    fn backup_descriptors(&self) -> Result<Vec<DescriptorsBackup>>;
    fn get_address(&self) -> Result<String>;
    fn list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>>;
    fn list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>>;
    fn feed_account_xpubs(&mut self, account_xpubs: &[AccountXPub]) -> Result<()>;
    fn list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>;
    fn set_heritage_config(&mut self, new_hc: &HeritageConfig) -> Result<()>;
    fn sync(&mut self) -> Result<()>;
    fn get_balance(&self) -> Result<HeritageWalletBalance>;
    fn last_sync_ts(&self) -> Result<u64>;
    fn create_psbt(
        &self,
        spending_config: SpendingConfig,
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
                AnyWalletOnline::None => Err(Error::MissingOnlineComponent),
                AnyWalletOnline::Service(sb) => sb.$fn_name($($a),*),
                AnyWalletOnline::Local(lhe) => lhe.$fn_name($($a),*),
            }
    };
}

impl WalletOnline for AnyWalletOnline {
    impl_wallet_online_fn!(backup_descriptors(&self) -> Result<Vec<DescriptorsBackup>>);
    impl_wallet_online_fn!(get_address(&self) -> Result<String>);
    impl_wallet_online_fn!(list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>>);
    impl_wallet_online_fn!(list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>>);
    impl_wallet_online_fn!(feed_account_xpubs(&mut self, account_xpubs: &[AccountXPub]) -> Result<()>);
    impl_wallet_online_fn!(list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>);
    impl_wallet_online_fn!(set_heritage_config(&mut self, new_hc: &HeritageConfig) -> Result<()>);
    impl_wallet_online_fn!(sync(&mut self) -> Result<()>);
    impl_wallet_online_fn!(get_balance(&self) -> Result<HeritageWalletBalance>);
    impl_wallet_online_fn!(last_sync_ts(&self) -> Result<u64>);
    impl_wallet_online_fn!(create_psbt(&self, spending_config: SpendingConfig) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}

impl crate::wallet::WalletCommons for AnyWalletOnline {
    impl_wallet_online_fn!(fingerprint(&self) -> Result<Option<Fingerprint>>);
    impl_wallet_online_fn!(network(&self) -> Result<Network> );
}
