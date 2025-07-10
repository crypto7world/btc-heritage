use crate::{
    errors::{Error, Result},
    BoundFingerprint, Broadcaster,
};
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, FeeRate, Txid},
    heritage_config::HeritageConfig,
    heritage_wallet::WalletAddress,
    AccountXPub, BlockInclusionObjective, HeritageWalletBackup, HeritageWalletBalance,
    PartiallySignedTransaction,
};

mod local;
mod service;

use heritage_service_api_client::{
    AccountXPubWithStatus, HeritageUtxo, HeritageWalletMeta, NewTx, SubwalletConfigMeta,
    TransactionSummary,
};
pub use local::{AnyBlockchainFactory, AuthConfig, BlockchainProviderConfig, LocalHeritageWallet};
use serde::{Deserialize, Serialize};
pub use service::ServiceBinding;

/// Status information for a heritage wallet
///
/// This struct contains comprehensive status information about a heritage wallet,
/// including its identity, balance, synchronization state, and fee information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletStatus {
    /// BIP32 master key fingerprint identifying this wallet
    pub fingerprint: Option<Fingerprint>,
    /// Current wallet balance breakdown
    pub balance: HeritageWalletBalance,
    /// Timestamp of the last successful synchronization (Unix timestamp)
    pub last_sync_ts: u64,
    /// Target number of blocks for transaction confirmation
    pub block_inclusion_objective: BlockInclusionObjective,
    /// Most recently used fee rate for transactions
    #[serde(default)]
    pub last_fee_rate: Option<FeeRate>,
}

impl From<HeritageWalletMeta> for WalletStatus {
    fn from(hwm: HeritageWalletMeta) -> Self {
        WalletStatus {
            fingerprint: hwm.fingerprint,
            balance: hwm.balance.unwrap_or_default(),
            last_sync_ts: hwm.last_sync_ts,
            block_inclusion_objective: hwm.block_inclusion_objective.unwrap_or_default(),
            last_fee_rate: hwm.fee_rate,
        }
    }
}

/// This trait regroups the functions of a Heritage wallet that do not need
/// access to the private keys and can be safely operated in an online environment.
///
/// The `OnlineWallet` trait provides all wallet operations that require network
/// connectivity but not private key access. This includes balance queries, address
/// generation, transaction history, and PSBT creation. It extends both `Broadcaster`
/// and `BoundFingerprint` traits to provide complete online wallet functionality.
pub trait OnlineWallet: Broadcaster + BoundFingerprint {
    fn backup_descriptors(
        &self,
    ) -> impl std::future::Future<Output = Result<HeritageWalletBackup>> + Send;
    fn get_address(&self) -> impl std::future::Future<Output = Result<WalletAddress>> + Send;
    fn list_addresses(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<WalletAddress>>> + Send;
    fn list_transactions(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<TransactionSummary>>> + Send;
    fn list_heritage_utxos(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<HeritageUtxo>>> + Send;
    fn list_account_xpubs(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<AccountXPubWithStatus>>> + Send;
    fn feed_account_xpubs(
        &mut self,
        account_xpubs: Vec<AccountXPub>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
    fn list_subwallet_configs(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<SubwalletConfigMeta>>> + Send;
    #[deprecated = "use list_subwallet_configs instead"]
    fn list_heritage_configs(
        &self,
    ) -> impl std::future::Future<Output = Result<Vec<HeritageConfig>>> + Send;
    fn set_heritage_config(
        &mut self,
        new_hc: HeritageConfig,
    ) -> impl std::future::Future<Output = Result<HeritageConfig>> + Send;
    fn sync(&mut self) -> impl std::future::Future<Output = Result<()>> + Send;
    fn get_wallet_status(&self) -> impl std::future::Future<Output = Result<WalletStatus>> + Send;
    fn set_block_inclusion_objective(
        &mut self,
        bio: BlockInclusionObjective,
    ) -> impl std::future::Future<Output = Result<WalletStatus>> + Send;
    fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> impl std::future::Future<Output = Result<(PartiallySignedTransaction, TransactionSummary)>> + Send;
}

/// Enumeration of all possible online wallet implementations
///
/// This enum provides a unified interface for different online wallet backends,
/// allowing the application to work with local wallets, remote service wallets,
/// or no wallet at all.
#[derive(Debug, Serialize, Deserialize)]
pub enum AnyOnlineWallet {
    /// No wallet is configured
    None,
    /// Remote Heritage Service-based wallet implementation
    Service(ServiceBinding),
    /// Local file-based wallet implementation
    Local(LocalHeritageWallet),
}

impl AnyOnlineWallet {
    /// Checks if the wallet is in the `None` state
    ///
    /// # Returns
    ///
    /// Returns `true` if no wallet is configured, `false` otherwise.
    pub fn is_none(&self) -> bool {
        match self {
            AnyOnlineWallet::None => true,
            _ => false,
        }
    }
}

/// Macro to implement OnlineWallet trait methods for AnyOnlineWallet
///
/// This macro generates the boilerplate code needed to dispatch trait method calls
/// to the appropriate underlying wallet implementation. It handles both mutable
/// and immutable method signatures and automatically returns errors for the None variant.
macro_rules! impl_online_wallet_fn {
    ($(#[$attrs:meta])? $fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        $(#[$attrs])?
        async fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            impl_online_wallet_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($(#[$attrs:meta])? $fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        $(#[$attrs])?
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            impl_online_wallet_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($self:ident $fn_name:ident($($a:ident : $t:ty),*)) => {
            match $self {
                AnyOnlineWallet::None => Err(Error::MissingOnlineWallet),
                AnyOnlineWallet::Service(sb) => sb.$fn_name($($a),*).await,
                AnyOnlineWallet::Local(lhe) => lhe.$fn_name($($a),*).await,
            }
    };
}

impl OnlineWallet for AnyOnlineWallet {
    impl_online_wallet_fn!(backup_descriptors(&self) -> Result<HeritageWalletBackup>);
    impl_online_wallet_fn!(get_address(&self) -> Result<WalletAddress>);
    impl_online_wallet_fn!(list_addresses(&self) -> Result<Vec<WalletAddress>>);
    impl_online_wallet_fn!(list_transactions(&self) -> Result<Vec<TransactionSummary>>);
    impl_online_wallet_fn!(list_heritage_utxos(&self) -> Result<Vec<HeritageUtxo>>);
    impl_online_wallet_fn!(list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>>);
    impl_online_wallet_fn!(feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()>);
    impl_online_wallet_fn!(list_subwallet_configs(&self) -> Result<Vec<SubwalletConfigMeta>>);
    impl_online_wallet_fn!(
        #[allow(deprecated)]
        list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>);
    impl_online_wallet_fn!(set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig>);
    impl_online_wallet_fn!(sync(&mut self) -> Result<()>);
    impl_online_wallet_fn!(get_wallet_status(&self) -> Result<WalletStatus>);
    impl_online_wallet_fn!(set_block_inclusion_objective(&mut self, bio: BlockInclusionObjective) -> Result<WalletStatus>);
    impl_online_wallet_fn!(create_psbt(&self, spending_config: NewTx) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}
impl Broadcaster for AnyOnlineWallet {
    impl_online_wallet_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
impl BoundFingerprint for AnyOnlineWallet {
    fn fingerprint(&self) -> Result<Fingerprint> {
        match self {
            AnyOnlineWallet::None => Err(Error::MissingOnlineWallet),
            AnyOnlineWallet::Service(sb) => sb.fingerprint(),
            AnyOnlineWallet::Local(lhe) => lhe.fingerprint(),
        }
    }
}

/// Macro to implement OnlineWallet trait for types that contain an AnyOnlineWallet
///
/// This macro generates a complete OnlineWallet implementation that delegates all
/// calls to an inner `online_wallet` field of type `AnyOnlineWallet`. It also
/// provides accessor methods for the online wallet field.
macro_rules! impl_online_wallet {
    ($(#[$attrs:meta])? $fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        $(#[$attrs])?
        async fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            $self.online_wallet.$fn_name($($a),*).await
        }
    };
    ($(#[$attrs:meta])? $fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        $(#[$attrs])?
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.online_wallet.$fn_name($($a),*).await
        }
    };
    ($name:ident) => {
        impl $name {
            pub fn online_wallet(&self) -> &AnyOnlineWallet {
                &self.online_wallet
            }
            pub fn online_wallet_mut(&mut self) -> &mut AnyOnlineWallet {
                &mut self.online_wallet
            }
        }
        impl OnlineWallet for $name {
            crate::online_wallet::impl_online_wallet!(backup_descriptors(&self) -> Result<btc_heritage::HeritageWalletBackup>);
            crate::online_wallet::impl_online_wallet!(get_address(&self) -> Result<btc_heritage::heritage_wallet::WalletAddress>);
            crate::online_wallet::impl_online_wallet!(list_addresses(&self) -> Result<Vec<btc_heritage::heritage_wallet::WalletAddress>>);
            crate::online_wallet::impl_online_wallet!(list_transactions(&self) -> Result<Vec<btc_heritage::heritage_wallet::TransactionSummary>>);
            crate::online_wallet::impl_online_wallet!(list_heritage_utxos(&self) -> Result<Vec<btc_heritage::heritage_wallet::HeritageUtxo>>);
            crate::online_wallet::impl_online_wallet!(list_account_xpubs(&self) -> Result<Vec<heritage_service_api_client::AccountXPubWithStatus>>);
            crate::online_wallet::impl_online_wallet!(feed_account_xpubs(&mut self, account_xpubs: Vec<btc_heritage::AccountXPub>) -> Result<()>);
            crate::online_wallet::impl_online_wallet!(list_subwallet_configs(&self) -> Result<Vec<heritage_service_api_client::SubwalletConfigMeta>>);
            crate::online_wallet::impl_online_wallet!(
                #[allow(deprecated)]
                list_heritage_configs(&self) -> Result<Vec<btc_heritage::HeritageConfig>>
            );
            crate::online_wallet::impl_online_wallet!(set_heritage_config(&mut self, new_hc: btc_heritage::HeritageConfig) -> Result<btc_heritage::HeritageConfig>);
            crate::online_wallet::impl_online_wallet!(sync(&mut self) -> Result<()>);
            crate::online_wallet::impl_online_wallet!(get_wallet_status(&self) -> Result<crate::online_wallet::WalletStatus>);
            crate::online_wallet::impl_online_wallet!(set_block_inclusion_objective(&mut self, bio: btc_heritage::BlockInclusionObjective) -> Result<crate::online_wallet::WalletStatus>);
            crate::online_wallet::impl_online_wallet!(create_psbt(&self, new_tx: heritage_service_api_client::NewTx) -> Result<(btc_heritage::PartiallySignedTransaction, btc_heritage::heritage_wallet::TransactionSummary)>);
        }
        impl crate::Broadcaster for $name {
            crate::online_wallet::impl_online_wallet!(broadcast(&self, psbt: btc_heritage::PartiallySignedTransaction) -> Result<btc_heritage::bitcoin::Txid>);
        }
    };
}
pub(crate) use impl_online_wallet;
