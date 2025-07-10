use std::{
    io::{stdout, Write},
    sync::Arc,
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

/// A binding to a remote heritage service wallet
///
/// This struct represents a connection to a heritage wallet managed by a remote service.
/// It maintains the wallet ID, cached fingerprint, and network configuration needed to
/// interact with the remote wallet through the heritage service API.
///
/// The service client is not serialized and must be reinitialized after deserialization.

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceBinding {
    /// Unique identifier for the wallet on the remote service
    wallet_id: Arc<str>,
    /// Cached BIP32 master key fingerprint for performance
    fingerprint: Option<Fingerprint>,
    /// Bitcoin network this wallet operates on
    network: Network,
    /// Service client for API communication (not serialized)
    #[serde(skip, default)]
    service_client: Option<HeritageServiceClient>,
}
impl ServiceBinding {
    /// Creates a new wallet on the remote service
    ///
    /// This method creates a new heritage wallet on the remote service with the given
    /// parameters. If a backup is provided, it will be used to restore the wallet state.
    ///
    /// # Arguments
    ///
    /// * `wallet_name` - Name for the new wallet
    /// * `backup` - Optional wallet backup to restore from
    /// * `block_inclusion_objective` - Target number of blocks for transaction confirmation
    /// * `service_client` - Authenticated service client for API communication
    /// * `network` - Bitcoin network for the wallet
    ///
    /// # Returns
    ///
    /// Returns a new `ServiceBinding` instance connected to the created wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if the wallet creation request fails.
    pub async fn create(
        wallet_name: &str,
        backup: Option<HeritageWalletBackup>,
        block_inclusion_objective: BlockInclusionObjective,
        service_client: HeritageServiceClient,
        network: Network,
    ) -> Result<Self> {
        let create = HeritageWalletMetaCreate {
            name: wallet_name.to_owned(),
            backup,
            block_inclusion_objective: Some(block_inclusion_objective),
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
    /// Creates a service binding from existing wallet metadata
    ///
    /// This is a private helper method that creates a ServiceBinding from
    /// wallet metadata retrieved from the service.
    ///
    /// # Arguments
    ///
    /// * `wallet` - Wallet metadata from the service
    /// * `service_client` - Authenticated service client
    /// * `network` - Bitcoin network for the wallet
    ///
    /// # Returns
    ///
    /// Returns a new `ServiceBinding` instance.
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
    /// Binds to an existing wallet on the service by name
    ///
    /// This method searches for an existing wallet with the given name and
    /// creates a binding to it. The wallet name must be unique on the service.
    ///
    /// # Arguments
    ///
    /// * `existing_wallet_name` - Name of the existing wallet to bind to
    /// * `service_client` - Authenticated service client for API communication
    /// * `network` - Bitcoin network for the wallet
    ///
    /// # Returns
    ///
    /// Returns a `ServiceBinding` instance connected to the existing wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No wallet with the given name is found
    /// - Multiple wallets with the same name are found
    /// - The service request fails
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
    /// Binds to an existing wallet on the service by ID
    ///
    /// This method connects to an existing wallet using its unique ID.
    /// This is the most direct way to bind to a specific wallet.
    ///
    /// # Arguments
    ///
    /// * `existing_wallet_id` - Unique ID of the existing wallet
    /// * `service_client` - Authenticated service client for API communication
    /// * `network` - Bitcoin network for the wallet
    ///
    /// # Returns
    ///
    /// Returns a `ServiceBinding` instance connected to the existing wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No wallet with the given ID is found
    /// - The service request fails
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
    /// Binds to an existing wallet on the service by fingerprint
    ///
    /// This method searches for an existing wallet with the given BIP32 master
    /// key fingerprint and creates a binding to it. The fingerprint must be
    /// unique on the service.
    ///
    /// # Arguments
    ///
    /// * `existing_wallet_fingerprint` - BIP32 fingerprint of the existing wallet
    /// * `service_client` - Authenticated service client for API communication
    /// * `network` - Bitcoin network for the wallet
    ///
    /// # Returns
    ///
    /// Returns a `ServiceBinding` instance connected to the existing wallet.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No wallet with the given fingerprint is found
    /// - Multiple wallets with the same fingerprint are found
    /// - The service request fails
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
    /// Initializes the service client with fingerprint validation
    ///
    /// This method sets up the service client and validates that the remote
    /// wallet's fingerprint matches the locally cached one (if available).
    /// This helps ensure we're connecting to the correct wallet.
    ///
    /// # Arguments
    ///
    /// * `service_client` - Authenticated service client for API communication
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The remote wallet's fingerprint doesn't match the local cache
    /// - The service request fails
    pub async fn init_service_client(
        &mut self,
        service_client: HeritageServiceClient,
    ) -> Result<()> {
        if service_client
            .get_wallet(&self.wallet_id)
            .await?
            .fingerprint
            != self.fingerprint
            && self.fingerprint.is_some()
        {
            return Err(Error::IncoherentServiceWalletFingerprint);
        }
        unsafe {
            self.init_service_client_unchecked(service_client);
        }
        Ok(())
    }
    /// Synchronizes the local fingerprint with the remote wallet fingerprint
    ///
    /// Fetches the fingerprint from the remote service and updates the local
    /// fingerprint if they differ.
    ///
    /// # Returns
    ///
    /// Returns `true` if the fingerprint was updated, `false` if it was already
    /// in sync.
    ///
    /// # Errors
    ///
    /// Returns an error if the service client is not initialized or if the
    /// remote wallet cannot be retrieved.
    pub(crate) async fn sync_fingerprint(&mut self) -> Result<bool> {
        let remote_fg = self
            .unwrap_service_client()?
            .get_wallet(&self.wallet_id)
            .await?
            .fingerprint;
        if self.fingerprint != remote_fg {
            self.fingerprint = remote_fg;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// Initializes the service client without validation
    ///
    /// This method sets up the service client without performing any
    /// fingerprint validation. This bypasses the security check that ensures
    /// the remote wallet matches the expected fingerprint.
    ///
    /// # Arguments
    ///
    /// * `service_client` - Service client for API communication
    ///
    /// # Safety
    ///
    /// This function is marked as unsafe because it bypasses the fingerprint
    /// validation that ensures the service client connects to the correct wallet.
    /// The caller must ensure that:
    ///
    /// - The service client is authenticated to the correct account
    /// - The wallet in the service is the expected one
    ///
    /// Using this function incorrectly could result in connecting to the wrong
    /// wallet or performing operations on unintended accounts.
    pub unsafe fn init_service_client_unchecked(&mut self, service_client: HeritageServiceClient) {
        self.service_client = Some(service_client);
    }
    /// Checks if a service client is initialized
    ///
    /// # Returns
    ///
    /// Returns `true` if a service client is available, `false` otherwise.
    pub fn has_service_client(&self) -> bool {
        self.service_client.is_some()
    }
    /// Gets a reference to the service client
    ///
    /// # Returns
    ///
    /// Returns an optional reference to the service client.
    pub fn service_client(&self) -> Option<&HeritageServiceClient> {
        self.service_client.as_ref()
    }
    /// Gets a reference to the service client or returns an error
    ///
    /// # Returns
    ///
    /// Returns a reference to the service client.
    ///
    /// # Errors
    ///
    /// Returns an error if no service client is initialized.
    fn unwrap_service_client(&self) -> Result<&HeritageServiceClient> {
        self.service_client
            .as_ref()
            .ok_or(Error::UninitializedServiceClient)
    }
    /// Gets the wallet ID
    ///
    /// # Returns
    ///
    /// Returns the unique identifier of the wallet on the remote service.
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

    async fn set_block_inclusion_objective(
        &mut self,
        bio: BlockInclusionObjective,
    ) -> Result<super::WalletStatus> {
        Ok(self
            .unwrap_service_client()?
            .patch_wallet(&self.wallet_id, None, Some(bio))
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
    /// Broadcasts a signed transaction through the remote service
    ///
    /// This method sends the signed PSBT to the remote service for broadcasting
    /// to the Bitcoin network.
    ///
    /// # Arguments
    ///
    /// * `psbt` - The signed partial transaction to broadcast
    ///
    /// # Returns
    ///
    /// Returns the transaction ID of the broadcasted transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The service client is not initialized
    /// - The broadcast request fails
    async fn broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid> {
        Ok(self
            .unwrap_service_client()?
            .post_broadcast_tx(psbt)
            .await?)
    }
}

impl BoundFingerprint for ServiceBinding {
    /// Returns the BIP32 master key fingerprint
    ///
    /// # Returns
    ///
    /// Returns the cached fingerprint of the wallet's master key.
    ///
    /// # Errors
    ///
    /// Returns an error if the fingerprint is not available.
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self
            .fingerprint
            .ok_or(Error::OnlineWalletFingerprintNotPresent)?)
    }
}
