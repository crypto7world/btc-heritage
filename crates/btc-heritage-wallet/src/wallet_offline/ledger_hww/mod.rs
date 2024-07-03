use std::{fmt::Debug, ops::Deref};

use crate::errors::Result;

use btc_heritage::bitcoin::{bip32::Fingerprint, Network};
use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    BitcoinClient, Transport,
};
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};
use serde::{Deserialize, Serialize};

/// Transport with the Ledger device.
struct TransportHID(TransportNativeHID);

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
    #[serde(skip, default)]
    ledger_client: LedgerClient,
}

impl super::WalletOffline for LedgerKey {
    fn sign_psbt(&self, psbt: &mut btc_heritage::PartiallySignedTransaction) -> Result<usize> {
        todo!()
    }

    fn derive_accounts_xpubs(
        &self,
        count: usize,
    ) -> Result<Vec<btc_heritage::miniscript::DescriptorPublicKey>> {
        todo!()
    }

    fn derive_heir_xpub(&self) -> Result<btc_heritage::miniscript::DescriptorPublicKey> {
        todo!()
    }
}

impl crate::wallet::WalletCommons for LedgerKey {
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        todo!()
    }

    fn network(&self) -> crate::errors::Result<Network> {
        todo!()
    }
}
