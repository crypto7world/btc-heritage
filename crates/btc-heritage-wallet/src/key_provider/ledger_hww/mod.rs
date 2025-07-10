//! Ledger hardware wallet integration module.
//!
//! This module provides integration with Ledger hardware wallets for secure Bitcoin operations.
//! It handles device communication, policy registration, and signing operations while maintaining
//! compatibility with the heritage wallet system.

use core::{fmt::Debug, ops::Deref, str::FromStr};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

use crate::{
    errors::{Error, Result},
    BoundFingerprint, LedgerPolicy,
};

use btc_heritage::{
    account_xpub::AccountXPubId,
    bitcoin::{
        bip32::{ChildNumber, DerivationPath, Fingerprint},
        key::XOnlyPublicKey,
        taproot::{Signature, TapLeafHash},
        Network,
    },
    AccountXPub,
};
use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    psbt::PartialSignature,
    BitcoinClient, Transport, WalletPolicy,
};
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use policy::{LedgerPolicyHMAC, LedgerPolicyId};
use serde::{Deserialize, Serialize};

use super::MnemonicBackup;

pub(crate) mod policy;

/// Global static mutex containing an optional reference-counted LedgerClient instance
/// that can be shared throughout the application.
///
/// This singleton pattern ensures that we have a valid LedgerClient pointing to an
/// currently connected the Ledger device at any time. The mutex provides
/// thread-safe updates to this singleton.
static LEDGER_CLIENT: Mutex<Option<LedgerClient>> = Mutex::new(None);

/// Provides access to the LedgerClient singleton, creating it if needed.
///
/// This function manages the lifecycle of a shared LedgerClient instance:
/// - If a valid client already exists, returns it with its fingerprint
/// - If the existing client has become invalid, removes it and returns None
/// - If no client exists, attempts to create one
///
/// # Returns
///
/// Some((client, fingerprint)) if a valid LedgerClient is available, otherwise None.
///
/// # Examples
///
/// ```ignore
/// if let Some((client, fingerprint)) = ledger_client().await {
///     // Use client and fingerprint
/// } else {
///     // Handle missing or invalid Ledger device
/// }
/// ```
pub async fn ledger_client() -> Option<(LedgerClient, Fingerprint)> {
    let opt_ledger_client = { LEDGER_CLIENT.lock().unwrap().clone() };
    match opt_ledger_client {
        Some(lc) => match lc.fingerprint().await {
            Ok(fg) => Some((lc.clone(), fg)),
            Err(e) => {
                log::error!("{e}");
                LEDGER_CLIENT.lock().unwrap().take();
                None
            }
        },
        None => match LedgerClient::new() {
            Ok(lc) => {
                if let Ok(fg) = lc.fingerprint().await {
                    LEDGER_CLIENT.lock().unwrap().replace(lc.clone());
                    Some((lc, fg))
                } else {
                    None
                }
            }
            Err(e) => {
                log::debug!("{e}");
                None
            }
        },
    }
}

/// Transport layer for communicating with Ledger devices via HID (Human Interface Device).
///
/// This struct wraps the native HID transport to provide a unified interface for
/// communicating with Ledger hardware wallets. It handles low-level APDU command
/// exchange and status code processing.
pub struct TransportHID(TransportNativeHID);

impl Debug for TransportHID {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("TransportHID").finish()
    }
}

impl TransportHID {
    /// Creates a new HID transport wrapper.
    ///
    /// # Arguments
    ///
    /// * `t` - The native HID transport instance to wrap
    pub fn new(t: TransportNativeHID) -> Self {
        Self(t)
    }
}

impl Transport for TransportHID {
    type Error = crate::errors::Error;

    /// Exchanges APDU commands with the Ledger device.
    ///
    /// This method handles the conversion between different APDU command formats
    /// and processes the response from the device.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The APDU command to send to the device
    ///
    /// # Returns
    ///
    /// A tuple containing the status word and response data from the device
    ///
    /// # Errors
    ///
    /// Returns an error if the transport fails to communicate with the device
    fn exchange(&self, cmd: &APDUCommand) -> Result<(StatusWord, Vec<u8>)> {
        // Convert to ledger_apdu format and exchange with device
        self.0
            .exchange(&ledger_apdu::APDUCommand {
                ins: cmd.ins,
                cla: cmd.cla,
                p1: cmd.p1,
                p2: cmd.p2,
                data: cmd.data.clone(),
            })
            .map(|answer| {
                (
                    // Convert response code to StatusWord, defaulting to Unknown if invalid
                    StatusWord::try_from(answer.retcode()).unwrap_or(StatusWord::Unknown),
                    answer.data().to_vec(),
                )
            })
            .map_err(crate::errors::Error::generic)
    }
}

/// A client for communicating with Ledger Bitcoin applications.
///
/// This struct provides a high-level interface for interacting with Ledger hardware
/// wallets, wrapping the underlying BitcoinClient and providing async operations.
/// It uses Arc for shared ownership and thread-safe cloning.
pub struct LedgerClient(Arc<BitcoinClient<TransportHID>>);

impl Debug for LedgerClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("LedgerClient").finish()
    }
}

impl Clone for LedgerClient {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl LedgerClient {
    /// Creates a new Ledger client instance.
    ///
    /// This method initializes the HID API and establishes a connection to the
    /// first available Ledger device.
    ///
    /// # Errors
    ///
    /// Returns an error if no Ledger device is found or if the HID API fails to initialize
    fn new() -> Result<Self> {
        Ok(Self(Arc::new(BitcoinClient::new(TransportHID::new(
            TransportNativeHID::new(&HidApi::new().expect("unable to get HIDAPI"))
                .map_err(|e| Error::LedgerClientError(e.to_string()))?,
        )))))
    }

    /// Retrieves the master fingerprint from the Ledger device.
    ///
    /// This method runs the fingerprint retrieval in a blocking task to avoid
    /// blocking the async runtime, as the underlying Ledger client is synchronous.
    ///
    /// # Returns
    ///
    /// The BIP32 fingerprint of the master key on the device
    ///
    /// # Errors
    ///
    /// Returns an error if the device communication fails or if the fingerprint
    /// cannot be retrieved
    pub async fn fingerprint(&self) -> Result<Fingerprint> {
        let ledger_client = self.clone();

        let fingerprint =
            tokio::task::spawn_blocking(move || ledger_client.get_master_fingerprint())
                .await
                .unwrap()
                .map(|fg| {
                    // Convert between different rust-bitcoin versions used by BDK and ledger_bitcoin_client
                    Fingerprint::from(fg.as_bytes())
                })?;
        log::debug!("LedgerClient::fingerprint => {fingerprint}");
        Ok(fingerprint)
    }
    /// Retrieves the Bitcoin network information from the Ledger device.
    ///
    /// This method queries the Ledger device to determine which Bitcoin application
    /// is currently running and maps it to the corresponding network type. The operation
    /// is executed in a blocking task to avoid blocking the async runtime.
    ///
    /// # Returns
    ///
    /// The Bitcoin network type (mainnet, testnet, etc.) based on the application
    /// running on the Ledger device
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Device communication fails
    /// - An unsupported or unknown Bitcoin application is running
    /// - The device is not available
    pub async fn network(&self) -> Result<Network> {
        let ledger_client = self.clone();
        let (name, version, flags) =
            tokio::task::spawn_blocking(move || ledger_client.get_version())
                .await
                .unwrap()?;
        log::debug!(
            "LedgerClient::get_version => name: {name}, version: {version}, flags: {flags:?}"
        );
        match name.as_str() {
            "Bitcoin" => Ok(Network::Bitcoin),
            "Bitcoin Test" => Ok(Network::Testnet),
            _ => Err(Error::WrongLedgerApplication),
        }
    }
}

impl Deref for LedgerClient {
    type Target = BitcoinClient<TransportHID>;

    /// Provides direct access to the underlying BitcoinClient.
    ///
    /// This allows the LedgerClient to be used wherever a BitcoinClient is expected.
    fn deref(&self) -> &Self::Target {
        &self.0.deref()
    }
}

/// A key provider implementation for Ledger hardware wallets.
///
/// This struct manages the connection to a Ledger device and handles key operations
/// such as signing transactions and deriving extended public keys. It maintains
/// a registry of wallet policies that have been registered with the device.
#[derive(Debug, Serialize, Deserialize)]
pub struct LedgerKey {
    /// The BIP32 fingerprint of the expected Ledger device
    fingerprint: Fingerprint,
    /// The Bitcoin network this key provider operates on
    network: Network,
    /// Map of registered wallet policies indexed by account ID
    /// Each entry contains the policy, its ID on the device, and the HMAC for authentication
    #[serde(default)]
    registered_policies: HashMap<AccountXPubId, (LedgerPolicy, LedgerPolicyId, LedgerPolicyHMAC)>,
}

impl LedgerKey {
    /// Creates a new LedgerKey instance for the specified network.
    ///
    /// This method connects to the first available Ledger device and retrieves
    /// its fingerprint to bind this key provider to that specific device.
    ///
    /// # Arguments
    ///
    /// * `network` - The Bitcoin network to operate on (mainnet, testnet, etc.)
    ///
    /// # Returns
    ///
    /// A new LedgerKey instance bound to the connected device
    ///
    /// # Errors
    ///
    /// Returns an error if no Ledger device is available or if the connection fails
    pub async fn new(network: Network) -> Result<Self> {
        let (_, fingerprint) = ledger_client().await.ok_or(Error::NoLedgerDevice)?;
        Ok(Self {
            fingerprint,
            network,
            registered_policies: HashMap::new(),
        })
    }

    /// Executes a function with the Ledger client, ensuring device consistency.
    ///
    /// This method provides a safe way to execute operations on the Ledger device
    /// by verifying that the connected device matches the one this key provider
    /// was initialized with. The operation is executed in a blocking task to avoid
    /// blocking the async runtime.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes a LedgerClient and returns a result
    ///
    /// # Returns
    ///
    /// The result of the function execution
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No Ledger device is available
    /// - The connected device fingerprint doesn't match the expected one
    /// - The function execution fails
    pub(crate) async fn ledger_call<
        R: Send + 'static,
        F: FnOnce(
                LedgerClient,
            ) -> core::result::Result<
                R,
                ledger_bitcoin_client::error::BitcoinClientError<crate::errors::Error>,
            > + Send
            + 'static,
    >(
        &self,
        f: F,
    ) -> Result<R> {
        let (client, fingerprint) = ledger_client().await.ok_or(Error::NoLedgerDevice)?;

        // Ensure the connected device matches the one we're bound to
        if fingerprint != self.fingerprint {
            return Err(Error::IncoherentLedgerWalletFingerprint);
        }

        // Execute the function in a blocking task to avoid blocking the async runtime
        Ok(tokio::task::spawn_blocking(move || f(client))
            .await
            .unwrap()?)
    }

    /// Registers a list of wallet policies with the Ledger device.
    ///
    /// This method takes a list of heritage wallet policies and registers them
    /// with the connected Ledger device. Each policy must be registered before
    /// it can be used for signing operations. The registration process generates
    /// a unique ID and HMAC for each policy.
    ///
    /// # Arguments
    ///
    /// * `policies` - A vector of LedgerPolicy instances to register
    /// * `progress` - A callback function called for each policy during registration
    ///
    /// # Returns
    ///
    /// The number of policies that were newly registered (not previously registered)
    ///
    /// # Errors
    ///
    /// Returns an error if the device communication fails or if policy registration fails
    pub async fn register_policies<P>(
        &mut self,
        policies: &Vec<LedgerPolicy>,
        progress: P,
    ) -> Result<usize>
    where
        P: Fn(&WalletPolicy),
    {
        let mut register_results = Vec::with_capacity(policies.len());

        for policy in policies {
            let account_id = policy.get_account_id();
            let wallet_policy: WalletPolicy = policy.into();

            // Call the progress callback to allow UI updates
            progress(&wallet_policy);

            // Register the policy with the device
            let (id, hmac) = self
                .ledger_call(move |client| client.register_wallet(&wallet_policy))
                .await?;
            register_results.push((
                account_id,
                (
                    policy.clone(),
                    LedgerPolicyId::from(id),
                    LedgerPolicyHMAC::from(hmac),
                ),
            ));
        }

        let before = self.registered_policies.len();
        self.registered_policies
            .extend(register_results.into_iter());
        Ok(self.registered_policies.len() - before)
    }

    /// Lists all wallet policies currently registered with this LedgerKey instance.
    ///
    /// This method returns a vector containing all the policies that have been
    /// registered with the Ledger device, along with their associated metadata.
    ///
    /// # Returns
    ///
    /// A vector of tuples containing:
    /// - Account ID
    /// - The wallet policy
    /// - The policy ID assigned by the device
    /// - The HMAC for policy authentication
    pub fn list_registered_policies(
        &self,
    ) -> Vec<(
        AccountXPubId,
        LedgerPolicy,
        LedgerPolicyId,
        LedgerPolicyHMAC,
    )> {
        self.registered_policies
            .iter()
            .map(|(account_id, (p, id, hmac))| (*account_id, p.clone(), id.clone(), hmac.clone()))
            .collect()
    }
}

impl super::KeyProvider for LedgerKey {
    async fn sign_psbt(
        &self,
        psbt: &mut btc_heritage::PartiallySignedTransaction,
    ) -> Result<usize> {
        // Extract account IDs from the PSBT inputs by examining tap_key_origins
        // We look for derivation paths that match our fingerprint and extract the account index
        let account_ids_present: HashSet<AccountXPubId> = psbt
            .inputs
            .iter()
            .map(|input| {
                input
                    .tap_key_origins
                    .iter()
                    .filter_map(|(_, (_, (fg, dp)))| {
                        // Only consider keys that belong to our device
                        if fg == &self.fingerprint {
                            // Extract account index from derivation path (third element)
                            match dp[2] {
                                ChildNumber::Normal { .. } => None,
                                ChildNumber::Hardened { index } => Some(index),
                            }
                        } else {
                            None
                        }
                    })
            })
            .flatten()
            .collect();

        // Ensure all required policies are registered
        if !account_ids_present
            .iter()
            .all(|i| self.registered_policies.contains_key(i))
        {
            return Err(Error::LedgerMissingRegisteredPolicy(
                account_ids_present.into_iter().collect(),
            ));
        }

        // Convert PSBT format between different rust-bitcoin versions
        // This is necessary because BDK uses a different version than ledger_bitcoin_client
        let psbt_v_ledger = bitcoin::Psbt::deserialize(
            &btc_heritage::bitcoin::psbt::PartiallySignedTransaction::serialize(&psbt),
        )
        .map_err(Error::generic)?;

        let mut signed_inputs = 0;

        // Sign with each registered policy that has inputs in the PSBT
        for account_id in account_ids_present {
            let (pol, _, hmac) = self
                .registered_policies
                .get(&account_id)
                .expect("we ensured every ids are in the Hashtable");

            let psbt_v_ledger_clone = psbt_v_ledger.clone();
            let ledger_pol = pol.into();
            let ledger_hmac = Some((*hmac).into());

            // Execute signing operation on the device
            let ret = self
                .ledger_call(move |client| {
                    client.sign_psbt(&psbt_v_ledger_clone, &ledger_pol, ledger_hmac.as_ref())
                })
                .await?;

            // Process the signatures returned by the device
            for (index, sig) in ret {
                signed_inputs += 1;
                match sig {
                    // Handle key path signatures (direct taproot key spending)
                    PartialSignature::Sig(key, sig) => {
                        log::debug!("index: {}, key: {}, sig: {}", index, key, sig);
                        let sig: Signature =
                            Signature::from_slice(&sig.to_vec()).expect("same underlying data");
                        psbt.inputs[index].tap_key_sig = Some(sig);
                    }
                    // Handle script path signatures (tapscript spending)
                    PartialSignature::TapScriptSig(key, tapleaf_hash, sig) => {
                        log::debug!(
                            "index: {}, key: {}, tapleaf_hash: {:?}, sig: {:?}",
                            index,
                            key,
                            tapleaf_hash,
                            sig.to_vec()
                        );
                        // Convert types between different rust-bitcoin versions
                        let key: XOnlyPublicKey = XOnlyPublicKey::from_str(&key.to_string())
                            .expect("same underlying data");
                        let tapleaf_hash = tapleaf_hash.map(|tapleaf_hash| {
                            TapLeafHash::from_str(&tapleaf_hash.to_string())
                                .expect("same underlying data")
                        });
                        let sig: Signature =
                            Signature::from_slice(&sig.to_vec()).expect("same underlying data");
                        match tapleaf_hash {
                            Some(tapleaf_hash) => {
                                // Script path signature with specific tapleaf
                                psbt.inputs[index]
                                    .tap_script_sigs
                                    .insert((key, tapleaf_hash), sig);
                            }
                            None => {
                                // Key path signature
                                psbt.inputs[index].tap_key_sig = Some(sig);
                            }
                        };
                    }
                }
            }
        }
        Ok(signed_inputs)
    }

    /// Derives extended public keys for a range of account indices.
    ///
    /// This method generates account-level extended public keys from the Ledger device
    /// using the BIP86 derivation path (m/86'/cointype'/account'). Each account XPub
    /// can be used to derive addresses for that specific account.
    ///
    /// # Arguments
    ///
    /// * `range` - The range of account indices to derive (e.g., 0..5 for accounts 0-4)
    ///
    /// # Returns
    ///
    /// A vector of AccountXPub instances, one for each account in the range
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The Ledger device is not available
    /// - Any account index is out of bounds (>= 2^31)
    /// - Device communication fails
    /// - Key derivation fails
    async fn derive_accounts_xpubs(
        &self,
        range: core::ops::Range<u32>,
    ) -> Result<Vec<AccountXPub>> {
        // Determine coin type based on network (0 for mainnet, 1 for testnet/regtest)
        let cointype_path_segment = match self.network {
            Network::Bitcoin => 0,
            _ => 1,
        };

        // Build the base derivation path for BIP86 (Taproot): m/86'/cointype'/
        let base_derivation_path = vec![
            ChildNumber::from_hardened_idx(86).unwrap(), // BIP86 purpose
            ChildNumber::from_hardened_idx(cointype_path_segment).unwrap(), // coin type
        ];
        let base_derivation_path = DerivationPath::from(base_derivation_path);

        let mut xpubs = Vec::with_capacity(range.len());

        // Derive an extended public key for each account in the range
        for i in range {
            // Create full derivation path: m/86'/cointype'/account'
            let derivation_path = base_derivation_path.extend([ChildNumber::from_hardened_idx(i)
                .map_err(|_| Error::AccountDerivationIndexOutOfBound(i))?]);

            // Convert between different rust-bitcoin versions for compatibility
            let ledger_derivation_path =
                bitcoin::bip32::DerivationPath::from_str(&derivation_path.to_string())
                    .map_err(Error::generic)?;

            // Request the extended public key from the device
            let xpub: bitcoin::bip32::Xpub = self
                .ledger_call(move |client| {
                    client.get_extended_pubkey(&ledger_derivation_path, false)
                })
                .await?;
            let derivation_path_str = derivation_path.to_string();

            // Format as descriptor public key with fingerprint and derivation path
            let desc_pub_key = format!(
                "[{}/{}]{}/*",
                self.fingerprint,
                &derivation_path_str[2..], // Skip "m/" prefix
                xpub
            );
            log::debug!("{derivation_path_str} from Ledger: {desc_pub_key}");
            xpubs.push(AccountXPub::try_from(desc_pub_key.as_str())?)
        }
        Ok(xpubs)
    }

    /// Attempts to derive an heir configuration from the Ledger device.
    ///
    /// This method is not supported for Ledger devices as they do not expose
    /// the seed phrase or allow derivation of heir-specific keys for security reasons.
    ///
    /// # Arguments
    ///
    /// * `_heir_config_type` - The type of heir configuration requested (ignored)
    ///
    /// # Errors
    ///
    /// Always returns `Error::LedgerHeirUnsupported` as this operation is not
    /// supported on hardware wallets
    async fn derive_heir_config(
        &self,
        _heir_config_type: super::HeirConfigType,
    ) -> Result<btc_heritage::HeirConfig> {
        Err(Error::LedgerHeirUnsupported)
    }

    /// Attempts to backup the mnemonic seed phrase from the Ledger device.
    ///
    /// This method is not supported for Ledger devices as they do not expose
    /// their seed phrases for security reasons. The seed phrase remains secure
    /// within the hardware device.
    ///
    /// # Errors
    ///
    /// Always returns `Error::LedgerBackupMnemonicUnsupported` as this operation
    /// is not supported on hardware wallets
    async fn backup_mnemonic(&self) -> Result<MnemonicBackup> {
        Err(Error::LedgerBackupMnemonicUnsupported)
    }
}

impl BoundFingerprint for LedgerKey {
    /// Returns the BIP32 fingerprint of the bound Ledger device.
    ///
    /// This fingerprint uniquely identifies the hardware wallet and is used
    /// to ensure operations are performed on the correct device.
    ///
    /// # Returns
    ///
    /// The fingerprint of the Ledger device this key provider is bound to
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
