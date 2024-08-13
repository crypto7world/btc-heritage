use core::{fmt::Debug, ops::Deref, str::FromStr};
use std::collections::{HashMap, HashSet};

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

/// Transport with the Ledger device.
pub(crate) struct TransportHID(TransportNativeHID);
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

struct LedgerClient(BitcoinClient<TransportHID>);
impl Debug for LedgerClient {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("LedgerClient").finish()
    }
}
impl LedgerClient {
    pub fn new() -> Result<Self> {
        Ok(Self(BitcoinClient::new(TransportHID::new(
            TransportNativeHID::new(&HidApi::new().expect("unable to get HIDAPI"))
                .map_err(|e| Error::LedgerClientError(e.to_string()))?,
        ))))
    }
}

impl Deref for LedgerClient {
    type Target = BitcoinClient<TransportHID>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LedgerKey {
    fingerprint: Fingerprint,
    network: Network,
    #[serde(default)]
    registered_policies: HashMap<AccountXPubId, (LedgerPolicy, LedgerPolicyId, LedgerPolicyHMAC)>,
    #[serde(skip, default)]
    ledger_client: Option<LedgerClient>,
}

impl LedgerKey {
    pub fn new(network: Network) -> Result<Self> {
        let ledger_client = Some(LedgerClient::new()?);
        let fingerprint = ledger_client.as_ref().unwrap().get_master_fingerprint()?;
        Ok(Self {
            // Because for now we are bound to the rust-bitcoin version of BDK
            // which is different than the one used by ledger_bitcoin_client
            fingerprint: Fingerprint::from(fingerprint.as_bytes()),
            network,
            registered_policies: HashMap::new(),
            ledger_client,
        })
    }
    pub fn init_ledger_client(&mut self) -> Result<()> {
        self.ledger_client = Some(LedgerClient::new()?);

        if self
            .ledger_client
            .as_ref()
            .unwrap()
            .get_master_fingerprint()?
            .as_bytes()
            != self.fingerprint.as_bytes()
        {
            return Err(Error::IncoherentLedgerWalletFingerprint);
        }
        Ok(())
    }
    fn ledger_client(&self) -> &LedgerClient {
        self.ledger_client
            .as_ref()
            .expect("ledger client should have been initialized")
    }
    pub fn register_policies(&mut self, policies: &Vec<LedgerPolicy>) -> Result<usize> {
        let client = self.ledger_client();
        let register_results = policies
            .iter()
            .map(|policy| {
                let account_id = policy.get_account_id();
                let wallet_policy: WalletPolicy = policy.into();
                let (id, hmac) = client.register_wallet(&wallet_policy)?;
                Ok::<_, Error>((
                    account_id,
                    (
                        policy.clone(),
                        LedgerPolicyId::from(id),
                        LedgerPolicyHMAC::from(hmac),
                    ),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
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
    fn sign_psbt(&self, psbt: &mut btc_heritage::PartiallySignedTransaction) -> Result<usize> {
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
            let ret =
                self.ledger_client()
                    .sign_psbt(&psbt_v_ledger, &pol.into(), Some(hmac.into()))?;
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

    fn derive_accounts_xpubs(&self, range: core::ops::Range<u32>) -> Result<Vec<AccountXPub>> {
        let cointype_path_segment = match self.network {
            Network::Bitcoin => 0,
            _ => 1,
        };
        let base_derivation_path = vec![
            ChildNumber::from_hardened_idx(86).unwrap(),
            ChildNumber::from_hardened_idx(cointype_path_segment).unwrap(),
        ];
        let base_derivation_path = DerivationPath::from(base_derivation_path);

        let xpubs = range
            .into_iter()
            .map(|i| {
                let derivation_path = base_derivation_path
                    .extend([ChildNumber::from_hardened_idx(i)
                        .map_err(|_| Error::AccountDerivationIndexOutOfBound(i))?]);
                let xpub: bitcoin::bip32::Xpub = self.ledger_client().get_extended_pubkey(
                    // Because for now we are bound to the rust-bitcoin version of BDK
                    // which is different than the one used by ledger_bitcoin_client
                    &bitcoin::bip32::DerivationPath::from_str(&derivation_path.to_string())
                        .map_err(Error::generic)?,
                    false,
                )?;
                let derivation_path_str = derivation_path.to_string();

                let desc_pub_key = format!(
                    "[{}/{}]{}/*",
                    self.fingerprint,
                    &derivation_path_str[2..],
                    xpub
                );
                log::debug!("{derivation_path_str} from Ledger: {desc_pub_key}");
                Ok(AccountXPub::try_from(desc_pub_key.as_str())?)
            })
            .collect();
        xpubs
    }

    fn derive_heir_config(
        &self,
        _heir_config_type: super::HeirConfigType,
    ) -> Result<btc_heritage::HeirConfig> {
        Err(Error::LedgerHeirUnsupported)
    }

    fn backup_mnemonic(&self) -> Result<MnemonicBackup> {
        Err(Error::LedgerBackupMnemonicUnsupported)
    }
}

impl BoundFingerprint for LedgerKey {
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
