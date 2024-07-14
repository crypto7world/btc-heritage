use std::{
    io::{stdout, Write},
    thread,
    time::Duration,
};

use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Network},
    utils::timestamp_now,
};
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
        let wallet_id = service_client.post_wallets(wallet_name)?.id;
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
        self.service_client()
            .get_wallet_descriptors_backup(&self.wallet_id)
    }

    fn get_address(&self) -> Result<String> {
        self.service_client()
            .post_wallet_create_address(&self.wallet_id)
    }

    fn list_account_xpubs(&self) -> Result<Vec<crate::service_client::AccountXPubWithStatus>> {
        self.service_client()
            .list_wallet_account_xpubs(&self.wallet_id)
    }
    fn feed_account_xpubs(&mut self, account_xpubs: Vec<btc_heritage::AccountXPub>) -> Result<()> {
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
        self.service_client()
            .list_wallet_heritage_configs(&self.wallet_id)
    }

    fn set_heritage_config(&mut self, new_hc: btc_heritage::HeritageConfig) -> Result<()> {
        self.service_client()
            .post_wallet_heritage_configs(&self.wallet_id, new_hc)?;
        Ok(())
    }

    fn sync(&mut self) -> Result<()> {
        // Get last_sync ts
        let last_sync_ts = self
            .service_client()
            .get_wallet(&self.wallet_id)?
            .last_sync_ts;
        // If sync is old, ask for a new sync and wait
        if last_sync_ts + 60 < timestamp_now() {
            print!("Syncing");
            let _ = stdout().flush();
            self.service_client()
                .post_wallet_synchronize(&self.wallet_id)?;
            loop {
                print!(".");
                let _ = stdout().flush();
                thread::sleep(Duration::from_secs(5));
                let new_last_sync_ts = self
                    .service_client()
                    .get_wallet(&self.wallet_id)?
                    .last_sync_ts;
                if last_sync_ts != new_last_sync_ts {
                    break;
                }
            }
            println!(".");
        }
        Ok(())
    }

    fn get_wallet_info(&self) -> Result<super::WalletInfo> {
        let hwm = self.service_client().get_wallet(&self.wallet_id)?;
        Ok(super::WalletInfo {
            fingerprint: hwm.fingerprint,
            balance: hwm.balance.unwrap_or_default(),
            last_sync_ts: hwm.last_sync_ts,
        })
    }

    fn create_psbt(
        &self,
        new_tx: crate::service_client::NewTx,
    ) -> Result<(
        btc_heritage::PartiallySignedTransaction,
        btc_heritage::heritage_wallet::TransactionSummary,
    )> {
        self.service_client()
            .post_wallet_create_unsigned_tx(&self.wallet_id, new_tx)
    }

    fn broadcast(
        &self,
        psbt: btc_heritage::PartiallySignedTransaction,
    ) -> Result<btc_heritage::bitcoin::Txid> {
        self.service_client().post_broadcast_tx(psbt)
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
