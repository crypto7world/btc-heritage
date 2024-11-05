use btc_heritage::{Amount, PartiallySignedTransaction};

use heritage_service_api_client::{
    Fingerprint, HeritageServiceClient, NewTxDrainTo, TransactionSummary,
};
use serde::{Deserialize, Serialize};

use crate::{errors::Result, BoundFingerprint, Broadcaster};

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
    fn service_client(&self) -> &HeritageServiceClient {
        self.service_client
            .as_ref()
            .expect("service client should have been initialized")
    }
}

impl super::HeritageProvider for ServiceBinding {
    fn list_heritages(&self) -> Result<Vec<Heritage>> {
        Ok(self
            .service_client()
            .list_heritages()?
            .into_iter()
            .filter_map(|api_h| {
                if api_h
                    .heir_config
                    .is_some_and(|hc| hc.fingerprint() == self.fingerprint)
                    && api_h.value.is_some()
                    && api_h.maturity.is_some()
                    && api_h.next_heir_maturity.is_some()
                {
                    Some(Heritage {
                        heritage_id: api_h.heritage_id,
                        value: Amount::from_sat(api_h.value.unwrap()),
                        maturity: api_h.maturity.unwrap(),
                        next_heir_maturity: api_h.next_heir_maturity.unwrap(),
                    })
                } else {
                    None
                }
            })
            .collect())
    }
    fn create_psbt(
        &self,
        heritage_id: &str,
        drain_to: btc_heritage::bitcoin::Address,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        Ok(self.service_client().post_heritage_create_unsigned_tx(
            heritage_id,
            NewTxDrainTo {
                drain_to: drain_to.to_string(),
            },
        )?)
    }
}

impl Broadcaster for ServiceBinding {
    fn broadcast(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> Result<heritage_service_api_client::Txid> {
        Ok(self.service_client().post_broadcast_tx(psbt)?)
    }
}

impl BoundFingerprint for ServiceBinding {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
