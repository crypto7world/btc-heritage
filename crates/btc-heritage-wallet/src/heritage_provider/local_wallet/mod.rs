use btc_heritage::PartiallySignedTransaction;

use heritage_service_api_client::{Fingerprint, TransactionSummary};

use serde::{Deserialize, Serialize};

use crate::{errors::Result, BoundFingerprint, Broadcaster};

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalWallet {
    fingerprint: Fingerprint,
    heritage_wallet_name: String,
}

impl super::HeritageProvider for LocalWallet {
    fn list_heritages(&self) -> Result<Vec<super::Heritage>> {
        todo!()
    }
    fn create_psbt(
        &self,
        heritage_id: &str,
        drain_to: btc_heritage::bitcoin::Address,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        todo!()
    }
}
impl Broadcaster for LocalWallet {
    fn broadcast(&self, psbt: PartiallySignedTransaction) -> Result<heritage_service_api_client::Txid> {
        todo!()
    }
}
impl BoundFingerprint for LocalWallet {
    fn fingerprint(&self) -> Result<Fingerprint> {
        todo!()
    }
}
