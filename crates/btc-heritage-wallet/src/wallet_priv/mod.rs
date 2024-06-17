use crate::errors::Result;
use btc_heritage::{
    bitcoin::bip32::Fingerprint, miniscript::DescriptorPublicKey, PartiallySignedTransaction,
};

mod ledger_hww;
mod local_key;
use ledger_hww::LedgerDevice;
use local_key::LocalKey;

/// This trait regroup the functions of an Heritage wallet that need
/// access to the private keys and that should be operated in an offline environment or using
/// a hardware-wallet device.
pub trait HeritageWalletPriv {
    /// Sign all the (Tap) inputs of the given PSBT that can be signed using the privates keys
    /// and return the number of inputs signed.
    fn sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>;
    /// Return a list of the first `count` account eXtended Public Keys as a [Vec<DescriptorPublicKey>]
    fn derive_accounts_xpubs(&self, count: usize) -> Result<Vec<DescriptorPublicKey>>;
    /// Return the [DescriptorPublicKey] of the heir account descriptor.
    /// By convention, it correspond to the account 1751476594 which is the decimal value corresponding
    /// to `u32::from_be_bytes(*b"heir")`.
    fn derive_heir_xpub(&self) -> Result<DescriptorPublicKey>;
    /// Return the [Fingerprint] of the underlying master private key
    fn fingerprint(&self) -> Result<Fingerprint>;
}

pub enum AnyHeritageWalletPriv {
    None,
    LocalKey(LocalKey),
    Ledger(LedgerDevice),
}
