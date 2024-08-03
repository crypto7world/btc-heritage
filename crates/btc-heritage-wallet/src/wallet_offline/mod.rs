use core::ops::Range;

use crate::errors::{Error, Result};
use bip39::Mnemonic;
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Network},
    AccountXPub, HeirConfig, PartiallySignedTransaction,
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

/// This trait regroup the functions of an Heritage wallet that need
/// access to the private keys and that should be operated in an offline environment or using
/// a hardware-wallet device.
pub trait WalletOffline {
    /// Sign all the (Tap) inputs of the given PSBT that can be signed using the privates keys
    /// and return the number of inputs signed.
    fn sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>;
    /// Return a list of the first `count` account eXtended Public Keys as a [Vec<AccountXPub>]
    fn derive_accounts_xpubs(&self, range: Range<u32>) -> Result<Vec<AccountXPub>>;
    /// Return an [HeirConfig] of the [HeirConfigType] asked for.
    /// Both [HeirConfigType::SingleHeirPubkey] and [HeirConfigType::HeirXPubkey] are taken from the account 1751476594 which is the decimal value corresponding
    /// to `u32::from_be_bytes(*b"heir")`.
    fn derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>;
    /// Return the [Mnemonic] of the Offline wallet.
    ///
    /// # Beware
    /// This is critical information. Assuming there is no password-protection,
    /// the mnemonic is enough to generate any and all wallet private keys
    fn get_mnemonic(&self) -> Result<Mnemonic>;
}

#[derive(Debug, Serialize, Deserialize)]
pub enum AnyWalletOffline {
    None,
    LocalKey(LocalKey),
    Ledger(LedgerKey),
}

impl AnyWalletOffline {
    pub fn is_none(&self) -> bool {
        match self {
            AnyWalletOffline::None => true,
            _ => false,
        }
    }
}

macro_rules! impl_wallet_offline_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            impl_wallet_offline_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            impl_wallet_offline_fn!($self $fn_name($($a : $t),*))
        }
    };
    ($self:ident $fn_name:ident($($a:ident : $t:ty),*)) => {
            match $self {
                AnyWalletOffline::None => Err(Error::MissingOnlineComponent),
                AnyWalletOffline::LocalKey(lk) => lk.$fn_name($($a),*),
                AnyWalletOffline::Ledger(ledger) => ledger.$fn_name($($a),*),
            }
    };
}

impl WalletOffline for AnyWalletOffline {
    impl_wallet_offline_fn!(sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>);
    impl_wallet_offline_fn!(derive_accounts_xpubs(&self, range: Range<u32>) -> Result<Vec<AccountXPub>>);
    impl_wallet_offline_fn!(derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>);
    impl_wallet_offline_fn!(get_mnemonic(&self) -> Result<Mnemonic>);
}

impl crate::wallet::WalletCommons for AnyWalletOffline {
    impl_wallet_offline_fn!(fingerprint(&self) -> Result<Option<Fingerprint>>);
    impl_wallet_offline_fn!(network(&self) -> Result<Network> );
}
