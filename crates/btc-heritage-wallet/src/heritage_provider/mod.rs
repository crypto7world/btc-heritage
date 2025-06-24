use crate::{
    errors::{Error, Result},
    BoundFingerprint, Broadcaster,
};
use btc_heritage::{
    bitcoin::{amount, bip32::Fingerprint, Address, Txid},
    heritage_wallet::TransactionSummary,
    Amount, HeirConfig, PartiallySignedTransaction,
};

use serde::{Deserialize, Serialize};

mod local;
mod service;
pub use local::LocalWallet;
pub use service::ServiceBinding;

type Timestamp = u64;

#[derive(Debug, Serialize, Deserialize)]
pub struct Heritage {
    pub heritage_id: String,
    /// The heir_config for which the following info are generated
    pub heir_config: HeirConfig,
    /// The value (correspond to the underlying UTXO)
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "amount::serde::as_sat::opt"
    )]
    pub value: Option<Amount>,
    /// The timestamp after which the Heir is able to spend
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maturity: Option<Timestamp>,
    /// The maturity of the next heir, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_heir_maturity: Option<Option<Timestamp>>,
    /// The position of the heir in the HeritageConfig
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heir_position: Option<u8>,
    /// The number of heirs in the HeritageConfig
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heirs_count: Option<u8>,
}

/// This trait regroup the functions of an Heritage wallet that does not need
/// access to the private keys and can be safely operated in an online environment.
pub trait HeritageProvider: Broadcaster + BoundFingerprint {
    /// List the [Heritage]s that can be spend with create_psbt
    fn list_heritages(&self) -> impl std::future::Future<Output = Result<Vec<Heritage>>> + Send;
    /// Create a PSBT draining all the [Heritage] that can be spend to a given [Address]
    fn create_psbt(
        &self,
        heritage_id: &str,
        drain_to: Address,
    ) -> impl std::future::Future<Output = Result<(PartiallySignedTransaction, TransactionSummary)>> + Send;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AnyHeritageProvider {
    None,
    Service(ServiceBinding),
    LocalWallet(LocalWallet),
}

impl AnyHeritageProvider {
    pub fn is_none(&self) -> bool {
        match self {
            AnyHeritageProvider::None => true,
            _ => false,
        }
    }
}

macro_rules! impl_heritage_provider_fn {
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            impl_heritage_provider_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($self:ident $fn_name:ident($($a:ident : $t:ty),*)) => {
            match $self {
                AnyHeritageProvider::None => Err(Error::MissingHeritageProvider),
                AnyHeritageProvider::Service(sb) => sb.$fn_name($($a),*).await,
                AnyHeritageProvider::LocalWallet(lw) => lw.$fn_name($($a),*).await,
            }
    };
}

impl HeritageProvider for AnyHeritageProvider {
    impl_heritage_provider_fn!(list_heritages(&self) -> Result<Vec<Heritage>>);
    impl_heritage_provider_fn!(create_psbt(&self, heritage_id: &str,drain_to: Address) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}

impl Broadcaster for AnyHeritageProvider {
    impl_heritage_provider_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
impl BoundFingerprint for AnyHeritageProvider {
    fn fingerprint(&self) -> Result<Fingerprint> {
        match self {
            AnyHeritageProvider::None => Err(Error::MissingHeritageProvider),
            AnyHeritageProvider::Service(sb) => sb.fingerprint(),
            AnyHeritageProvider::LocalWallet(lw) => lw.fingerprint(),
        }
    }
}

macro_rules! impl_heritage_provider {
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.heritage_provider.$fn_name($($a),*).await
        }
    };
    ($name:ident) => {
        impl $name {
            pub fn heritage_provider(&self) -> &AnyHeritageProvider {
                &self.heritage_provider
            }
            pub fn heritage_provider_mut(&mut self) -> &mut AnyHeritageProvider {
                &mut self.heritage_provider
            }
        }
        impl HeritageProvider for $name {
            crate::heritage_provider::impl_heritage_provider!(list_heritages(&self) -> Result<Vec<Heritage>>);
            crate::heritage_provider::impl_heritage_provider!(create_psbt(&self, heritage_id: &str,drain_to: btc_heritage::bitcoin::Address) -> Result<(btc_heritage::PartiallySignedTransaction, btc_heritage::heritage_wallet::TransactionSummary)>);
        }
        impl Broadcaster for $name {
            crate::heritage_provider::impl_heritage_provider!(broadcast(&self, psbt: btc_heritage::PartiallySignedTransaction) -> Result<btc_heritage::bitcoin::Txid>);
        }
    };
}
pub(crate) use impl_heritage_provider;
