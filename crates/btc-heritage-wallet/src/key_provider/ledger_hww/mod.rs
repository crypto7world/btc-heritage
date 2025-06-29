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

/// Transport with the Ledger device.
pub struct TransportHID(TransportNativeHID);
impl Debug for TransportHID {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("TransportHID").finish()
    }
}

impl TransportHID {
    pub fn new(t: TransportNativeHID) -> Self {
        Self(t)
    }
}

impl Transport for TransportHID {
    type Error = crate::errors::Error;
    fn exchange(&self, cmd: &APDUCommand) -> Result<(StatusWord, Vec<u8>)> {
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
                    StatusWord::try_from(answer.retcode()).unwrap_or(StatusWord::Unknown),
                    answer.data().to_vec(),
                )
            })
            .map_err(crate::errors::Error::generic)
    }
}

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
    fn new() -> Result<Self> {
        Ok(Self(Arc::new(BitcoinClient::new(TransportHID::new(
            TransportNativeHID::new(&HidApi::new().expect("unable to get HIDAPI"))
                .map_err(|e| Error::LedgerClientError(e.to_string()))?,
        )))))
    }
    pub async fn fingerprint(&self) -> Result<Fingerprint> {
        let ledger_client = self.clone();
        Ok(
            tokio::task::spawn_blocking(move || ledger_client.get_master_fingerprint())
                .await
                .unwrap()
                .map(|fg| {
                    // Because for now we are bound to the rust-bitcoin version of BDK
                    // which is different than the one used by ledger_bitcoin_client
                    Fingerprint::from(fg.as_bytes())
                })?,
        )
    }
}

impl Deref for LedgerClient {
    type Target = BitcoinClient<TransportHID>;
    fn deref(&self) -> &Self::Target {
        &self.0.deref()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LedgerKey {
    fingerprint: Fingerprint,
    network: Network,
    #[serde(default)]
    registered_policies: HashMap<AccountXPubId, (LedgerPolicy, LedgerPolicyId, LedgerPolicyHMAC)>,
}

impl LedgerKey {
    pub async fn new(network: Network) -> Result<Self> {
        let (_, fingerprint) = ledger_client().await.ok_or(Error::NoLedgerDevice)?;
        Ok(Self {
            fingerprint,
            network,
            registered_policies: HashMap::new(),
        })
    }

    // async fn ledger_client(&self) -> Result<LedgerClient> {
    //     let (client, fingerprint) = ledger_client().await.ok_or(Error::NoLedgerDevice)?;
    //     if fingerprint != self.fingerprint {
    //         return Err(Error::IncoherentLedgerWalletFingerprint);
    //     }
    //     Ok(client)
    // }

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
        if fingerprint != self.fingerprint {
            return Err(Error::IncoherentLedgerWalletFingerprint);
        }

        Ok(tokio::task::spawn_blocking(move || f(client))
            .await
            .unwrap()?)
    }

    pub async fn ledger_connected_and_ready(&self) -> bool {
        match self.ledger_call(|_| Ok(())).await {
            Ok(_) => true,
            Err(e) => {
                log::warn!("Ledger not ready: {e}");
                false
            }
        }
    }
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
            // Call the callback progress function so that the caller may display something
            progress(&wallet_policy);
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
        // We need to know what AccountXPubId are present in the PSBT inputs
        let account_ids_present: HashSet<AccountXPubId> = psbt
            .inputs
            .iter()
            .map(|input| {
                input
                    .tap_key_origins
                    .iter()
                    .filter_map(|(_, (_, (fg, dp)))| {
                        if fg == &self.fingerprint {
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
        if !account_ids_present
            .iter()
            .all(|i| self.registered_policies.contains_key(i))
        {
            return Err(Error::LedgerMissingRegisteredPolicy(
                account_ids_present.into_iter().collect(),
            ));
        }

        // Because for now we are bound to the rust-bitcoin version of BDK
        // which is different than the one used by ledger_bitcoin_client
        let psbt_v_ledger = bitcoin::Psbt::deserialize(
            &btc_heritage::bitcoin::psbt::PartiallySignedTransaction::serialize(&psbt),
        )
        .map_err(Error::generic)?;

        let mut signed_inputs = 0;

        for account_id in account_ids_present {
            let (pol, _, hmac) = self
                .registered_policies
                .get(&account_id)
                .expect("we ensured every ids are in the Hashtable");

            let psbt_v_ledger_clone = psbt_v_ledger.clone();
            let ledger_pol = pol.into();
            let ledger_hmac = Some((*hmac).into());
            let ret = self
                .ledger_call(move |client| {
                    client.sign_psbt(&psbt_v_ledger_clone, &ledger_pol, ledger_hmac.as_ref())
                })
                .await?;
            for (index, sig) in ret {
                signed_inputs += 1;
                match sig {
                    PartialSignature::Sig(key, sig) => {
                        log::debug!("index: {}, key: {}, sig: {}", index, key, sig);
                        let sig: Signature =
                            Signature::from_slice(&sig.to_vec()).expect("same underlying data");
                        psbt.inputs[index].tap_key_sig = Some(sig);
                    }
                    PartialSignature::TapScriptSig(key, tapleaf_hash, sig) => {
                        log::debug!(
                            "index: {}, key: {}, tapleaf_hash: {:?}, sig: {:?}",
                            index,
                            key,
                            tapleaf_hash,
                            sig.to_vec()
                        );
                        // Because for now we are bound to the rust-bitcoin version of BDK
                        // which is different than the one used by ledger_bitcoin_client
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
                                psbt.inputs[index]
                                    .tap_script_sigs
                                    .insert((key, tapleaf_hash), sig);
                            }
                            None => psbt.inputs[index].tap_key_sig = Some(sig),
                        };
                    }
                }
            }
        }
        Ok(signed_inputs)
    }

    async fn derive_accounts_xpubs(
        &self,
        range: core::ops::Range<u32>,
    ) -> Result<Vec<AccountXPub>> {
        let cointype_path_segment = match self.network {
            Network::Bitcoin => 0,
            _ => 1,
        };
        let base_derivation_path = vec![
            ChildNumber::from_hardened_idx(86).unwrap(),
            ChildNumber::from_hardened_idx(cointype_path_segment).unwrap(),
        ];
        let base_derivation_path = DerivationPath::from(base_derivation_path);

        let mut xpubs = Vec::with_capacity(range.len());
        for i in range {
            let derivation_path = base_derivation_path.extend([ChildNumber::from_hardened_idx(i)
                .map_err(|_| Error::AccountDerivationIndexOutOfBound(i))?]);

            // Because for now we are bound to the rust-bitcoin version of BDK
            // which is different than the one used by ledger_bitcoin_client
            let ledger_derivation_path =
                bitcoin::bip32::DerivationPath::from_str(&derivation_path.to_string())
                    .map_err(Error::generic)?;
            let xpub: bitcoin::bip32::Xpub = self
                .ledger_call(move |client| {
                    client.get_extended_pubkey(&ledger_derivation_path, false)
                })
                .await?;
            let derivation_path_str = derivation_path.to_string();

            let desc_pub_key = format!(
                "[{}/{}]{}/*",
                self.fingerprint,
                &derivation_path_str[2..],
                xpub
            );
            log::debug!("{derivation_path_str} from Ledger: {desc_pub_key}");
            xpubs.push(AccountXPub::try_from(desc_pub_key.as_str())?)
        }
        Ok(xpubs)
    }

    async fn derive_heir_config(
        &self,
        _heir_config_type: super::HeirConfigType,
    ) -> Result<btc_heritage::HeirConfig> {
        Err(Error::LedgerHeirUnsupported)
    }

    async fn backup_mnemonic(&self) -> Result<MnemonicBackup> {
        Err(Error::LedgerBackupMnemonicUnsupported)
    }
}

impl BoundFingerprint for LedgerKey {
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
