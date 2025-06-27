use btc_heritage::PartiallySignedTransaction;

use heritage_service_api_client::{
    Fingerprint, HeritageServiceClient, NewTxDrainTo, TransactionSummary,
};
use serde::{Deserialize, Serialize};

use crate::{
    errors::{Error, Result},
    BoundFingerprint, Broadcaster,
};

use super::Heritage;

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceBinding {
    fingerprint: Fingerprint,
    #[serde(skip, default)]
    service_client: Option<HeritageServiceClient>,
}
impl ServiceBinding {
    pub fn new(fingerprint: Fingerprint, service_client: HeritageServiceClient) -> Self {
        Self {
            fingerprint,
            service_client: Some(service_client),
        }
    }
    pub fn init_service_client(&mut self, service_client: HeritageServiceClient) {
        self.service_client = Some(service_client);
    }
    pub fn has_service_client(&self) -> bool {
        self.service_client.is_some()
    }
    fn service_client(&self) -> Result<&HeritageServiceClient> {
        self.service_client
            .as_ref()
            .ok_or(Error::UninitializedServiceClient)
    }
}

impl super::HeritageProvider for ServiceBinding {
    async fn list_heritages(&self) -> Result<Vec<Heritage>> {
        let client = self.service_client()?;
        let heritages = client.list_heritages().await?;
        Ok(heritages
            .into_iter()
            .filter_map(|api_h| {
                api_h
                    .heir_config
                    .as_ref()
                    .is_some_and(|hc| hc.fingerprint() == self.fingerprint)
                    .then(|| Heritage::from(api_h))
            })
            .collect())
    }
    async fn create_psbt(
        &self,
        heritage_id: &str,
        drain_to: btc_heritage::bitcoin::Address,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        Ok(self
            .service_client()?
            .post_heritage_create_unsigned_tx(
                heritage_id,
                NewTxDrainTo {
                    drain_to: drain_to.to_string(),
                },
            )
            .await?)
    }
}

impl Broadcaster for ServiceBinding {
    async fn broadcast(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> Result<heritage_service_api_client::Txid> {
        Ok(self.service_client()?.post_broadcast_tx(psbt).await?)
    }
}

impl BoundFingerprint for ServiceBinding {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
