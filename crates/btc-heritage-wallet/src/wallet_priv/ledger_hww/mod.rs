use crate::errors::Result;

use ledger_bitcoin_client::{
    apdu::{APDUCommand, StatusWord},
    BitcoinClient, Transport,
};
use ledger_transport_hid::{hidapi::HidApi, TransportNativeHID};

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

pub struct LedgerDevice(BitcoinClient<TransportHID>);

impl Default for LedgerDevice {
    fn default() -> Self {
        Self(BitcoinClient::new(TransportHID::new(
            TransportNativeHID::new(&HidApi::new().expect("unable to get HIDAPI")).unwrap(),
        )))
    }
}
