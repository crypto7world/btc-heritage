use core::ops::Range;

use crate::{
    errors::{Error, Result},
    BoundFingerprint,
};
use bip39::Mnemonic;
use btc_heritage::{
    bitcoin::bip32::Fingerprint, AccountXPub, HeirConfig, PartiallySignedTransaction,
};

pub(crate) mod ledger_hww;
pub(crate) mod local_key;
use ledger_hww::LedgerKey;
use local_key::LocalKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HeirConfigType {
    SingleHeirPubkey,
    HeirXPubkey,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MnemonicBackup {
    pub mnemonic: Mnemonic,
    pub fingerprint: Fingerprint,
    pub with_password: bool,
}

/// This trait regroup the functions of an Heritage wallet that need
/// access to the private keys and that should be operated in an offline environment or using
/// a hardware-wallet device.
pub trait KeyProvider: BoundFingerprint {
    /// Sign all the (Tap) inputs of the given PSBT that can be signed using the privates keys
    /// and return the number of inputs signed.
    fn sign_psbt(
        &self,
        psbt: &mut PartiallySignedTransaction,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Return a list of the first `count` account eXtended Public Keys as a [Vec<AccountXPub>]
    fn derive_accounts_xpubs(
        &self,
        range: Range<u32>,
    ) -> impl std::future::Future<Output = Result<Vec<AccountXPub>>> + Send;
    /// Return an [HeirConfig] of the [HeirConfigType] asked for.
    /// Both [HeirConfigType::SingleHeirPubkey] and [HeirConfigType::HeirXPubkey] are taken from the account 1751476594 which is the decimal value corresponding
    /// to `u32::from_be_bytes(*b"heir")`.
    fn derive_heir_config(
        &self,
        heir_config_type: HeirConfigType,
    ) -> impl std::future::Future<Output = Result<HeirConfig>> + Send;
    /// Return the [Mnemonic] of the Offline wallet.
    ///
    /// # Beware
    /// This is critical information. Assuming there is no password-protection,
    /// the mnemonic is enough to generate any and all wallet private keys
    fn backup_mnemonic(&self) -> impl std::future::Future<Output = Result<MnemonicBackup>> + Send;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AnyKeyProvider {
    None,
    LocalKey(LocalKey),
    Ledger(LedgerKey),
}

impl AnyKeyProvider {
    pub fn is_none(&self) -> bool {
        match self {
            AnyKeyProvider::None => true,
            _ => false,
        }
    }
    pub fn is_local(&self) -> bool {
        match self {
            AnyKeyProvider::LocalKey(_) => true,
            _ => false,
        }
    }
    pub fn is_ledger(&self) -> bool {
        match self {
            AnyKeyProvider::Ledger(_) => true,
            _ => false,
        }
    }
}

macro_rules! impl_key_provider_fn {
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            impl_key_provider_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($self:ident $fn_name:ident($($a:ident : $t:ty),*)) => {
            match $self {
                AnyKeyProvider::None => Err(Error::MissingKeyProvider),
                AnyKeyProvider::LocalKey(lk) => lk.$fn_name($($a),*).await,
                AnyKeyProvider::Ledger(ledger) => ledger.$fn_name($($a),*).await,
            }
    };
}

impl KeyProvider for AnyKeyProvider {
    impl_key_provider_fn!(sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>);
    impl_key_provider_fn!(derive_accounts_xpubs(&self, range: Range<u32>) -> Result<Vec<AccountXPub>>);
    impl_key_provider_fn!(derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>);
    impl_key_provider_fn!(backup_mnemonic(&self) -> Result<MnemonicBackup>);
}
impl BoundFingerprint for AnyKeyProvider {
    fn fingerprint(&self) -> Result<Fingerprint> {
        match self {
            AnyKeyProvider::None => Err(Error::MissingKeyProvider),
            AnyKeyProvider::LocalKey(lk) => lk.fingerprint(),
            AnyKeyProvider::Ledger(ledger) => ledger.fingerprint(),
        }
    }
}

macro_rules! impl_key_provider {
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        async fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.key_provider.$fn_name($($a),*).await
        }
    };
    ($name:ident$(<$lf:lifetime>)?) => {
        impl $name$(<$lf>)? {
            pub fn key_provider(&self) -> &AnyKeyProvider {
                &self.key_provider
            }
            pub fn key_provider_mut(&mut self) -> &mut AnyKeyProvider {
                &mut self.key_provider
            }
        }
        impl KeyProvider for $name$(<$lf>)? {
            crate::key_provider::impl_key_provider!(sign_psbt(&self, psbt: &mut btc_heritage::PartiallySignedTransaction) -> crate::errors::Result<usize>);
            crate::key_provider::impl_key_provider!(derive_accounts_xpubs(&self, range: core::ops::Range<u32>) -> crate::errors::Result<Vec<btc_heritage::AccountXPub>>);
            crate::key_provider::impl_key_provider!(derive_heir_config(&self, heir_config_type: crate::key_provider::HeirConfigType) -> crate::errors::Result<btc_heritage::HeirConfig>);
            crate::key_provider::impl_key_provider!(backup_mnemonic(&self) -> crate::errors::Result<crate::key_provider::MnemonicBackup>);
        }
    };
}
pub(crate) use impl_key_provider;
