use std::{
    io::{stdout, Write},
    sync::Arc,
    thread,
    time::Duration,
};

use crate::{
    database::dbitem::impl_db_single_item,
    errors::{Error, Result},
    BoundFingerprint, Broadcaster,
};
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Network, Txid},
    heritage_wallet::WalletAddress,
    AccountXPub, BlockInclusionObjective, HeritageConfig, HeritageWalletBackup,
    PartiallySignedTransaction,
};
use heritage_service_api_client::{
    AccountXPubWithStatus, HeritageServiceClient, HeritageServiceConfig, HeritageUtxo,
    HeritageWalletMeta, HeritageWalletMetaCreate, NewTx, SubwalletConfigMeta,
    SynchronizationStatus, TransactionSummary,
};

use serde::{Deserialize, Serialize};

impl_db_single_item!(HeritageServiceConfig, "heritage_service_configuration");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceBinding {
    wallet_id: Arc<str>,
    fingerprint: Option<Fingerprint>,
    network: Network,
    #[serde(skip, default)]
    service_client: Option<HeritageServiceClient>,
}
impl ServiceBinding {
    pub async fn create(
        wallet_name: &str,
        backup: Option<HeritageWalletBackup>,
        block_inclusion_objective: u16,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let create = HeritageWalletMetaCreate {
            name: wallet_name.to_owned(),
            backup,
            block_inclusion_objective: Some(BlockInclusionObjective::from(
                block_inclusion_objective,
            )),
        };
        let wallet_meta = service_client.post_wallets(create).await?;
        let wallet_id = wallet_meta.id;
        let fingerprint = wallet_meta.fingerprint;
        Ok(Self {
            wallet_id: wallet_id.into(),
            fingerprint,
            network,
            service_client: Some(service_client),
        })
    }
    fn bind(
        wallet: HeritageWalletMeta,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        Ok(Self {
            wallet_id: wallet.id.into(),
            fingerprint: wallet.fingerprint,
            network,
            service_client: Some(service_client),
        })
    }
    pub async fn bind_by_name(
        existing_wallet_name: &str,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallets = service_client.list_wallets().await?;
        let mut wallets = wallets
            .into_iter()
            .filter(|w| w.name == existing_wallet_name)
            .collect::<Vec<_>>();
        if wallets.len() == 0 {
            return Err(Error::NoServiceWalletFound);
        }
        if wallets.len() > 1 {
            return Err(Error::MultipleServiceWalletsFound);
        }
        ServiceBinding::bind(wallets.pop().unwrap(), service_client, network)
    }
    pub async fn bind_by_id(
        existing_wallet_id: &str,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallet = service_client
            .get_wallet(&existing_wallet_id)
            .await
            .map_err(|e| {
                log::error!("{e}");
                Error::NoServiceWalletFound
            })?;
        ServiceBinding::bind(wallet, service_client, network)
    }
    pub async fn bind_by_fingerprint(
        existing_wallet_fingerprint: Fingerprint,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let wallets = service_client.list_wallets().await?;
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
            return Err(Error::MultipleServiceWalletsFound);
        }
        ServiceBinding::bind(wallets.pop().unwrap(), service_client, network)
    }
    pub async fn init_service_client(
        &mut self,
        service_client: HeritageServiceClient,
    ) -> Result<()> {
        if service_client
            .get_wallet(&self.wallet_id)
            .await?
            .fingerprint
            != self.fingerprint
        {
            return Err(Error::IncoherentServiceWalletFingerprint);
        }
        self.init_service_client_unchecked(service_client);
        Ok(())
    }
    pub fn init_service_client_unchecked(&mut self, service_client: HeritageServiceClient) {
        self.service_client = Some(service_client);
    }
    pub fn has_service_client(&self) -> bool {
        self.service_client.is_some()
    }
    pub fn service_client(&self) -> Option<&HeritageServiceClient> {
        self.service_client.as_ref()
    }
    fn unwrap_service_client(&self) -> Result<&HeritageServiceClient> {
        self.service_client
            .as_ref()
            .ok_or(Error::UninitializedServiceClient)
    }
    pub fn wallet_id(&self) -> &str {
        &self.wallet_id
    }
}

impl super::OnlineWallet for ServiceBinding {
    async fn backup_descriptors(&self) -> Result<HeritageWalletBackup> {
        Ok(self
            .unwrap_service_client()?
            .get_wallet_descriptors_backup(&self.wallet_id)
            .await?)
    }

    async fn get_address(&self) -> Result<WalletAddress> {
        Ok(self
            .unwrap_service_client()?
            .post_wallet_create_address(&self.wallet_id)
            .await?)
    }

    async fn list_addresses(&self) -> Result<Vec<WalletAddress>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_addresses(&self.wallet_id)
            .await?)
    }

    async fn list_transactions(&self) -> Result<Vec<TransactionSummary>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_transactions(&self.wallet_id)
            .await?)
    }

    async fn list_heritage_utxos(&self) -> Result<Vec<HeritageUtxo>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_utxos(&self.wallet_id)
            .await?)
    }

    async fn list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_account_xpubs(&self.wallet_id)
            .await?)
    }
    async fn feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()> {
        let fingerprint = account_xpubs
            .get(0)
            .map(|axpub| axpub.descriptor_public_key().master_fingerprint());

        self.unwrap_service_client()?
            .post_wallet_account_xpubs(&self.wallet_id, account_xpubs)
            .await?;
        if self.fingerprint.is_none() {
            self.fingerprint = fingerprint;
        }
        Ok(())
    }

    async fn list_subwallet_configs(&self) -> Result<Vec<SubwalletConfigMeta>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_subwallet_configs(&self.wallet_id)
            .await?)
    }

    #[allow(deprecated)]
    async fn list_heritage_configs(&self) -> Result<Vec<HeritageConfig>> {
        Ok(self
            .unwrap_service_client()?
            .list_wallet_heritage_configs(&self.wallet_id)
            .await?)
    }

    async fn set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig> {
        Ok(self
            .unwrap_service_client()?
            .post_wallet_heritage_configs(&self.wallet_id, new_hc)
            .await?)
    }

    async fn set_block_inclusion_objective(&mut self, bio: u16) -> Result<super::WalletStatus> {
        Ok(self
            .unwrap_service_client()?
            .patch_wallet(
                &self.wallet_id,
                None,
                Some(BlockInclusionObjective::from(bio)),
            )
            .await?
            .into())
    }

    async fn sync(&mut self) -> Result<()> {
        // Ask for a sync
        let mut sync = self
            .unwrap_service_client()?
            .post_wallet_synchronize(&self.wallet_id)
            .await?;
        print!("Syncing");
        let _ = stdout().flush();
        loop {
            match sync.status {
                SynchronizationStatus::Queued | SynchronizationStatus::InProgress => {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    print!(".");
                    let _ = stdout().flush();
                }
                SynchronizationStatus::Ok => {
                    println!(".");
                    return Ok(());
                }
                SynchronizationStatus::Failed => {
                    return Err(Error::Generic("Synchronization failed".to_owned()))
                }
                SynchronizationStatus::Never => {
                    unreachable!("status from a sync request cannot be never")
                }
            }
            sync = self
                .unwrap_service_client()?
                .get_wallet_synchronize(&self.wallet_id)
                .await?;
        }
    }

    async fn get_wallet_status(&self) -> Result<super::WalletStatus> {
        let hwm = self
            .unwrap_service_client()?
            .get_wallet(&self.wallet_id)
            .await?;
        Ok(hwm.into())
    }

    async fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        Ok(self
            .unwrap_service_client()?
            .post_wallet_create_unsigned_tx(&self.wallet_id, new_tx)
            .await?)
    }
}

impl Broadcaster for ServiceBinding {
    async fn broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid> {
        Ok(self
            .unwrap_service_client()?
            .post_broadcast_tx(psbt)
            .await?)
    }
}

impl BoundFingerprint for ServiceBinding {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self
            .fingerprint
            .ok_or(Error::OnlineWalletFingerprintNotPresent)?)
    }
}
