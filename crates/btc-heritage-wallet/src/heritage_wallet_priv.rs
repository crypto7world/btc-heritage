use crate::errors::Result;
use btc_heritage::{miniscript::DescriptorPublicKey, PartiallySignedTransaction};

use crate::local_key::LocalKey;

use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    BitcoinClient, Transport,
};
use ledger_transport_hid::TransportNativeHID;

/// Transport with the Ledger device.
pub struct TransportHID(TransportNativeHID);

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

/// This trait regroup the functions of an Heritage wallet that need
/// access to the private keys and that should be operated in an offline environment or using
/// a hardware-wallet device.
trait HeritageWalletPriv {
    fn sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<bool>;
    fn derive_accounts_xpubs(&self, count: usize) -> Result<Vec<DescriptorPublicKey>>;
    fn derive_heir_xpub(&self) -> Result<DescriptorPublicKey>;
}

pub enum AnyHeritageWalletPriv {
    None,
    LocalKey(LocalKey),
    Ledger(BitcoinClient<TransportHID>),
}
