use btc_heritage::bitcoin::{bip32::Fingerprint, Network};
use serde::{Deserialize, Serialize};

use crate::{
    errors::{Error, Result},
    HeritageServiceClient,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServiceBinding {
    wallet_id: String,
    fingerprint: Option<Fingerprint>,
    network: Network,
    #[serde(skip, default)]
    service_client: Option<HeritageServiceClient>,
}
impl ServiceBinding {
    pub fn create(
        wallet_name: &str,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let fingerprint = None;
        let wallet_id = service_client.create_wallet(wallet_name)?.id;
        Ok(Self {
            wallet_id,
            fingerprint,
            network,
            service_client: Some(service_client),
        })
    }
    fn bind(
        wallet: crate::service_client::HeritageWalletMeta,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallet_id = wallet.id;
        Ok(Self {
            wallet_id,
            fingerprint: wallet.fingerprint,
            network,
            service_client: Some(service_client),
        })
    }
    pub fn bind_by_name(
        existing_wallet_name: &str,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallets = service_client.list_wallets()?;
        let mut wallets = wallets
            .into_iter()
            .filter(|w| w.name == existing_wallet_name)
            .collect::<Vec<_>>();
        if wallets.len() == 0 {
            return Err(Error::NoServiceWalletFound);
        }
        if wallets.len() > 1 {
            return Err(Error::MultipleServiceWalletFound);
        }
        ServiceBinding::bind(wallets.pop().unwrap(), service_client, network)
    }
    pub fn bind_by_id(
        existing_wallet_id: &str,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallet = service_client
            .get_wallet(&existing_wallet_id)
            .map_err(|e| {
                log::error!("{e}");
                Error::NoServiceWalletFound
            })?;
        ServiceBinding::bind(wallet, service_client, network)
    }
    pub fn bind_by_fingerprint(
        existing_wallet_fingerprint: Fingerprint,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallets = service_client.list_wallets()?;
        let mut wallets = wallets
            .into_iter()
            .filter(|w| {
                w.fingerprint
                    .is_some_and(|f| f == existing_wallet_fingerprint)
            })
            .collect::<Vec<_>>();
        if wallets.len() == 0 {
            return Err(Error::NoServiceWalletFound);
        }
        if wallets.len() > 1 {
            return Err(Error::MultipleServiceWalletFound);
        }
        ServiceBinding::bind(wallets.pop().unwrap(), service_client, network)
    }
    pub fn init_service_client(&mut self, service_client: HeritageServiceClient) -> Result<()> {
        self.service_client = Some(service_client);
        if self
            .service_client
            .as_ref()
            .unwrap()
            .get_wallet(&self.wallet_id)?
            .fingerprint
            != self.fingerprint
        {
            return Err(Error::IncoherentServiceWalletFingerprint);
        }
        Ok(())
    }
    fn service_client(&self) -> &HeritageServiceClient {
        self.service_client
            .as_ref()
            .expect("service client should have been initialized")
    }
}

impl super::WalletOnline for ServiceBinding {
    fn backup_descriptors(&self) -> Result<Vec<btc_heritage::heritage_wallet::DescriptorsBackup>> {
        todo!()
    }

    fn get_address(&self) -> Result<String> {
        todo!()
    }

    fn list_used_account_xpubs(&self) -> Result<Vec<btc_heritage::AccountXPub>> {
        todo!()
    }

    fn list_unused_account_xpubs(&self) -> Result<Vec<btc_heritage::AccountXPub>> {
        todo!()
    }

    fn feed_account_xpubs(&mut self, account_xpubs: &[btc_heritage::AccountXPub]) -> Result<()> {
        let fingerprint = if account_xpubs.len() > 0 {
            Some(
                account_xpubs[0]
                    .descriptor_public_key()
                    .master_fingerprint(),
            )
        } else {
            None
        };
        self.service_client()
            .post_wallet_account_xpubs(&self.wallet_id, account_xpubs)?;
        if self.fingerprint.is_none() {
            self.fingerprint = fingerprint;
        }
        Ok(())
    }

    fn list_heritage_configs(&self) -> Result<Vec<btc_heritage::HeritageConfig>> {
        todo!()
    }

    fn set_heritage_config(&mut self, new_hc: &btc_heritage::HeritageConfig) -> Result<()> {
        todo!()
    }

    fn sync(&mut self) -> Result<()> {
        todo!()
    }

    fn get_balance(&self) -> Result<btc_heritage::HeritageWalletBalance> {
        todo!()
    }

    fn last_sync_ts(&self) -> Result<u64> {
        todo!()
    }

    fn create_psbt(
        &self,
        spending_config: btc_heritage::SpendingConfig,
    ) -> Result<(
        btc_heritage::PartiallySignedTransaction,
        btc_heritage::heritage_wallet::TransactionSummary,
    )> {
        todo!()
    }
}

impl crate::wallet::WalletCommons for ServiceBinding {
    fn fingerprint(&self) -> Result<Option<Fingerprint>> {
        Ok(self
            .service_client()
            .get_wallet(&self.wallet_id)?
            .fingerprint)
    }

    fn network(&self) -> Result<btc_heritage::bitcoin::Network> {
        todo!()
    }
}
