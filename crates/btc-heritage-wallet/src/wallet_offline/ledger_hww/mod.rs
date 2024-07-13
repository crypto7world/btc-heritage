use std::{collections::HashMap, fmt::Debug, ops::Deref, str::FromStr};

use crate::{
    errors::{Error, Result},
    LedgerPolicy,
};

use btc_heritage::{
    bitcoin::{
        bip32::{ChildNumber, DerivationPath, Fingerprint},
        Network,
    },
    miniscript::DescriptorPublicKey,
    AccountXPub,
};
use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    BitcoinClient, Transport, WalletPolicy,
};
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use policy::{LedgerPolicyHMAC, LedgerPolicyId};
use serde::{Deserialize, Serialize};

pub(crate) mod policy;

/// Transport with the Ledger device.
pub(crate) struct TransportHID(TransportNativeHID);
impl Debug for TransportHID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
            .map_err(|e| crate::errors::Error::Generic(e.to_string()))
    }
}

struct LedgerClient(BitcoinClient<TransportHID>);
impl Debug for LedgerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LedgerClient").finish()
    }
}
impl Default for LedgerClient {
    fn default() -> Self {
        Self(BitcoinClient::new(TransportHID::new(
            TransportNativeHID::new(&HidApi::new().expect("unable to get HIDAPI")).unwrap(),
        )))
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
    registered_policies: HashMap<LedgerPolicy, (LedgerPolicyId, LedgerPolicyHMAC)>,
    #[serde(skip, default)]
    ledger_client: Option<LedgerClient>,
}

impl LedgerKey {
    pub fn new(network: Network) -> Result<Self> {
        let ledger_client = Some(LedgerClient::default());
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
        self.ledger_client = Some(LedgerClient::default());

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
                let wallet_policy: WalletPolicy = policy.into();
                let (id, hmac) = client.register_wallet(&wallet_policy)?;
                Ok::<_, Error>((
                    policy.clone(),
                    (
                        LedgerPolicyId::from(id.as_ref()),
                        LedgerPolicyHMAC::from(hmac.as_ref()),
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
    ) -> Vec<(LedgerPolicy, LedgerPolicyId, LedgerPolicyHMAC)> {
        self.registered_policies
            .iter()
            .map(|(p, (id, hmac))| (p.clone(), id.clone(), hmac.clone()))
            .collect()
    }
}

impl super::WalletOffline for LedgerKey {
    fn sign_psbt(&self, psbt: &mut btc_heritage::PartiallySignedTransaction) -> Result<usize> {
        todo!()
    }

    fn derive_accounts_xpubs(&self, count: usize) -> Result<Vec<AccountXPub>> {
        let cointype_path_segment = match self.network {
            Network::Bitcoin => 0,
            _ => 1,
        };
        let base_derivation_path = vec![
            ChildNumber::from_hardened_idx(86).unwrap(),
            ChildNumber::from_hardened_idx(cointype_path_segment).unwrap(),
        ];
        let base_derivation_path = DerivationPath::from(base_derivation_path);

        let xpubs: Result<Vec<AccountXPub>> = base_derivation_path
            .hardened_children()
            .take(count)
            .map(|derivation_path| {
                let xpub: bitcoin::bip32::Xpub = self.ledger_client().get_extended_pubkey(
                    // Because for now we are bound to the rust-bitcoin version of BDK
                    // which is different than the one used by ledger_bitcoin_client
                    &bitcoin::bip32::DerivationPath::from_str(&derivation_path.to_string())
                        .map_err(|e| Error::Generic(e.to_string()))?,
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

    fn derive_heir_xpub(&self) -> Result<DescriptorPublicKey> {
        todo!()
    }
}

impl crate::wallet::WalletCommons for LedgerKey {
    fn fingerprint(&self) -> crate::errors::Result<Option<Fingerprint>> {
        Ok(Some(self.fingerprint))
    }

    fn network(&self) -> crate::errors::Result<Network> {
        Ok(self.network)
    }
}
