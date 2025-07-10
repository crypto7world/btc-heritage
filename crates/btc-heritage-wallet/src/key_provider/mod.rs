//! Key provider module for managing private key operations in heritage wallets.
//!
//! This module provides abstractions for different types of key providers including
//! local software keys and hardware wallet integration. Key providers handle all
//! operations that require access to private keys and should be operated in secure
//! environments.
#![allow(
    deprecated,
    reason = "Else we get warning about HeirConfigType::SingleHeirPubkey because of the derivations"
)]
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

/// Configuration type for heir key derivation
///
/// Determines how heir keys are derived and stored in the heritage configuration.
/// Both types derive from the special account 1751476594 (decimal value of `u32::from_be_bytes(*b"heir")`).

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HeirConfigType {
    /// Use a single compressed public key for the heir
    ///
    /// This provides a specific public key that can be used directly in script templates.
    ///
    /// # Deprecated
    ///
    /// This variant is deprecated due to security and compatibility issues:
    /// - Reuses the same key repeatedly, which is suboptimal for privacy and security
    /// - Incompatible with Ledger hardware wallets
    /// - Use `HeirXPubkey` instead for better security and hardware wallet support
    #[deprecated(
        since = "0.25.0",
        note = "Use HeirXPubkey instead. SingleHeirPubkey reuses keys and is incompatible with Ledger devices"
    )]
    SingleHeirPubkey,
    /// Use an extended public key (xpub) for the heir
    ///
    /// This provides hierarchical deterministic key derivation capabilities, allowing
    /// the heir to generate multiple addresses from a single root key.
    HeirXPubkey,
}

/// Backup information for a mnemonic seed phrase
///
/// Contains the sensitive mnemonic data along with identifying information
/// to help restore wallet access. This structure holds critical wallet recovery
/// information and should be handled with extreme care.
#[derive(Debug, Serialize, Deserialize)]
pub struct MnemonicBackup {
    /// The BIP39 mnemonic seed phrase
    ///
    /// This is the master secret from which all wallet keys are derived.
    /// Loss of this mnemonic typically means permanent loss of wallet access.
    pub mnemonic: Mnemonic,
    /// The master key fingerprint derived from this mnemonic
    ///
    /// Used to verify master key derivation from the mnemonic when a password is used.
    pub fingerprint: Fingerprint,
    /// Whether the mnemonic was protected with a password
    ///
    /// If true, the mnemonic alone is insufficient for key derivation;
    /// the original password is also required for full wallet recovery.
    pub with_password: bool,
}

/// Provides private key operations for heritage wallets
///
/// This trait groups functions that require access to private keys and should be
/// operated in secure offline environments or using hardware wallet devices.
/// All key operations are asynchronous to accommodate hardware wallet interactions.
///
/// # Security Considerations
///
/// - Implementations handle sensitive cryptographic material
/// - Should be used in offline environments when possible
/// - Hardware wallet implementations provide additional security
/// - The `backup_mnemonic` method exposes critical recovery information
///
/// # Examples
///
/// ```ignore
/// # use btc_heritage_wallet::*;
/// # use std::ops::Range;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let key_provider = AnyKeyProvider::LocalKey(/* LocalKey instance */);
///
/// // Derive account extended public keys
/// let accounts = key_provider.derive_accounts_xpubs(0..5).await?;
///
/// // Generate heir configuration
/// let heir_config = key_provider.derive_heir_config(HeirConfigType::HeirXPubkey).await?;
/// # Ok(())
/// # }
/// ```
pub trait KeyProvider: BoundFingerprint {
    /// Sign all compatible inputs in a Partially Signed Bitcoin Transaction (PSBT)
    ///
    /// Attempts to sign all inputs in the PSBT that can be signed with the available
    /// private keys. Generally supports only Taproot (Schnorr) signatures.
    ///
    /// # Parameters
    ///
    /// * `psbt` - Mutable reference to the PSBT to sign
    ///
    /// # Returns
    ///
    /// The number of inputs that were successfully signed
    fn sign_psbt(
        &self,
        psbt: &mut PartiallySignedTransaction,
    ) -> impl std::future::Future<Output = Result<usize>> + Send;
    /// Derive a range of account extended public keys
    ///
    /// Generates account-level extended public keys (xpubs) for the specified range.
    /// Account xpubs are derived at the account level of the BIP44 derivation path
    /// and can be used to generate receive and change addresses.
    ///
    /// # Parameters
    ///
    /// * `range` - The range of account indices to derive (e.g., 0..5 for accounts 0-4)
    ///
    /// # Returns
    ///
    /// A vector of `AccountXPub` objects containing the derived public keys
    fn derive_accounts_xpubs(
        &self,
        range: Range<u32>,
    ) -> impl std::future::Future<Output = Result<Vec<AccountXPub>>> + Send;
    /// Derive heir configuration for inheritance setup
    ///
    /// Generates heir key material of the specified type from the special heir account.
    /// The heir account uses index 1751476594 (decimal value of `u32::from_be_bytes(*b"heir")`).
    ///
    /// # Parameters
    ///
    /// * `heir_config_type` - The type of heir configuration to generate
    ///
    /// # Returns
    ///
    /// An `HeirConfig` containing the appropriate key material for the heir
    fn derive_heir_config(
        &self,
        heir_config_type: HeirConfigType,
    ) -> impl std::future::Future<Output = Result<HeirConfig>> + Send;
    /// Backup the wallet's mnemonic seed phrase
    ///
    /// Retrieves the BIP39 mnemonic seed phrase and related backup information.
    /// This method provides access to the master secret from which all wallet
    /// keys are derived.
    ///
    /// # Returns
    ///
    /// A `MnemonicBackup` containing the mnemonic and related metadata
    ///
    /// # Security Warning
    ///
    /// This method exposes critical wallet recovery information. The mnemonic
    /// (combined with the password if any) is sufficient to generate all wallet
    /// private keys. Handle the returned data with extreme care:
    /// - Never store unencrypted in digital form
    /// - Prefer physical backup methods
    /// - Ensure secure disposal of any temporary copies
    fn backup_mnemonic(&self) -> impl std::future::Future<Output = Result<MnemonicBackup>> + Send;
}

/// Enum wrapper for different key provider implementations
///
/// Provides a unified interface for various key provider backends including
/// local software keys and hardware wallet integration. This allows the same
/// code to work with different key storage methods.
///
/// # Variants
///
/// * `None` - No key provider available (operations will fail)
/// * `LocalKey` - Software-based key storage using local cryptographic operations
/// * `Ledger` - Hardware wallet integration using Ledger devices
///
/// # Examples
///
/// ```ignore
/// # use btc_heritage_wallet::*;
/// // Create a local key provider
/// let local_provider = AnyKeyProvider::LocalKey(/* LocalKey instance */);
///
/// // Create a Ledger hardware wallet provider
/// let ledger_provider = AnyKeyProvider::Ledger(/* LedgerKey instance */);
///
/// // Check provider type
/// assert!(local_provider.is_local());
/// assert!(ledger_provider.is_ledger());
/// ```
#[derive(Debug, Serialize, Deserialize)]
pub enum AnyKeyProvider {
    /// No key provider available
    ///
    /// Operations requiring key access will return `Error::MissingKeyProvider`
    None,
    /// Local software-based key provider
    ///
    /// Stores and uses private keys locally with software cryptographic operations
    LocalKey(LocalKey),
    /// Ledger hardware wallet key provider
    ///
    /// Delegates key operations to a connected Ledger hardware wallet device
    Ledger(LedgerKey),
}

impl AnyKeyProvider {
    /// Check if the key provider is None
    ///
    /// # Returns
    ///
    /// `true` if this is the `None` variant, `false` otherwise
    pub fn is_none(&self) -> bool {
        match self {
            AnyKeyProvider::None => true,
            _ => false,
        }
    }

    /// Check if the key provider is a local software implementation
    ///
    /// # Returns
    ///
    /// `true` if this is the `LocalKey` variant, `false` otherwise
    pub fn is_local(&self) -> bool {
        match self {
            AnyKeyProvider::LocalKey(_) => true,
            _ => false,
        }
    }

    /// Check if the key provider is a Ledger hardware wallet
    ///
    /// # Returns
    ///
    /// `true` if this is the `Ledger` variant, `false` otherwise
    pub fn is_ledger(&self) -> bool {
        match self {
            AnyKeyProvider::Ledger(_) => true,
            _ => false,
        }
    }
}

/// Macro for implementing KeyProvider trait methods on AnyKeyProvider
///
/// This macro generates async method implementations that delegate to the
/// appropriate concrete key provider implementation or return an error
/// if no key provider is available.
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

/// Macro for implementing KeyProvider trait delegation for wallet types
///
/// This macro generates implementations that delegate KeyProvider trait methods
/// to an underlying `key_provider` field. It provides both the trait implementation
/// and accessor methods for the key provider.
///
/// # Usage
///
/// ```ignore
/// impl_key_provider!(WalletType);
/// ```
///
/// This generates:
/// - `key_provider()` and `key_provider_mut()` accessor methods
/// - Full `KeyProvider` trait implementation that delegates to the inner provider
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
