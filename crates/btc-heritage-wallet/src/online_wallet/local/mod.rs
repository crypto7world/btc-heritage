use std::{
    fmt::Debug,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    database::{blocking_db_operation, dbitem::impl_db_single_item, HeritageWalletDatabase},
    errors::{Error, Result},
    BoundFingerprint, Broadcaster, Database,
};
use btc_heritage::{
    bdk_types::{Auth, ElectrumBlockchain, RpcBlockchainFactory},
    bitcoin::{bip32::Fingerprint, secp256k1::rand, Txid},
    bitcoincore_rpc::{Client, RpcApi},
    database::HeritageDatabase,
    electrum_client::{self, ElectrumApi},
    heritage_wallet::{CreatePsbtOptions, SubwalletConfigId, TransactionSummary, WalletAddress},
    utils, AccountXPub, Amount, BlockInclusionObjective, HeritageConfig, HeritageWallet,
    HeritageWalletBackup, PartiallySignedTransaction, SpendingConfig,
};
use heritage_service_api_client::{
    AccountXPubWithStatus, NewTx, NewTxDrainTo, SubwalletConfigMeta,
};

use serde::{Deserialize, Serialize};

/// Authentication configuration for Bitcoin Core RPC connections
///
/// This enum defines the available authentication methods for connecting to a Bitcoin Core node.
/// It supports both cookie-based authentication and username/password authentication.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthConfig {
    /// Cookie file authentication
    ///
    /// Uses a .cookie file generated by Bitcoin Core for authentication.
    /// This is the default and recommended method for local connections.
    Cookie {
        /// Path to the cookie file
        file: Arc<str>,
    },
    /// Username/password authentication
    ///
    /// Uses a username and password combination for authentication.
    /// This method is typically used for remote connections or when
    /// cookie authentication is not available.
    UserPass {
        /// Username
        username: Arc<str>,
        /// Password
        password: Arc<str>,
    },
}
impl Default for AuthConfig {
    fn default() -> Self {
        let mut file: PathBuf = dirs_next::home_dir().unwrap_or_default();
        file.push(".bitcoin/.cookie");
        let file = file.to_str().expect("utf8 path").into();
        Self::Cookie { file }
    }
}

impl From<AuthConfig> for Auth {
    fn from(auth_config: AuthConfig) -> Self {
        match auth_config {
            AuthConfig::Cookie { file } => Auth::Cookie {
                file: PathBuf::from(file.as_ref()),
            },
            AuthConfig::UserPass { username, password } => Auth::UserPass {
                username: username.as_ref().to_owned(),
                password: password.as_ref().to_owned(),
            },
        }
    }
}

/// Configuration for different blockchain data providers
///
/// This enum defines the available blockchain backends that can be used
/// to synchronize wallet data and broadcast transactions. It supports
/// both Bitcoin Core RPC and Electrum server connections.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockchainProviderConfig {
    /// Bitcoin Core RPC connection configuration
    ///
    /// Connects directly to a Bitcoin Core node using RPC calls.
    /// This provides the most complete blockchain data access.
    BitcoinCore {
        /// RPC endpoint URL (e.g., "http://localhost:8332")
        url: Arc<str>,
        /// Authentication configuration for the RPC connection
        auth: AuthConfig,
    },
    /// Electrum server connection configuration
    ///
    /// Connects to an Electrum server for lightweight blockchain access.
    /// This is more bandwidth-efficient than Bitcoin Core but may have
    /// limited functionality.
    Electrum {
        /// Electrum server URL (e.g., "ssl://electrum.example.com:50002")
        url: Arc<str>,
    },
}

impl Default for BlockchainProviderConfig {
    fn default() -> Self {
        Self::BitcoinCore {
            url: "http://localhost:8332".into(),
            auth: AuthConfig::default(),
        }
    }
}

impl_db_single_item!(
    BlockchainProviderConfig,
    "blockchain_provider_configuration"
);

/// A unified blockchain factory that can create connections to different blockchain providers
///
/// This enum wraps the different blockchain factory types into a single interface,
/// allowing the wallet to work with either Bitcoin Core or Electrum backends
/// transparently.
#[derive(Clone)]
pub enum AnyBlockchainFactory {
    /// Bitcoin Core RPC blockchain factory
    Bitcoin(RpcBlockchainFactory),
    /// Electrum blockchain connection (wrapped in Arc for cloning)
    Electrum(Arc<ElectrumBlockchain>),
}
impl Debug for AnyBlockchainFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Bitcoin(_) => "Bitcoin(RpcBlockchainFactory)",
                Self::Electrum(_) => "Electrum(Arc<ElectrumBlockchain>)",
            }
        )
    }
}

impl TryFrom<BlockchainProviderConfig> for AnyBlockchainFactory {
    type Error = Error;

    fn try_from(bcpc: BlockchainProviderConfig) -> Result<Self> {
        let network = utils::bitcoin_network::get();
        Ok(match bcpc {
            BlockchainProviderConfig::BitcoinCore { url, auth } => {
                AnyBlockchainFactory::Bitcoin(RpcBlockchainFactory {
                    url: url.as_ref().to_owned(),
                    auth: Auth::from(auth),
                    network,
                    wallet_name_prefix: None,
                    default_skip_blocks: 0,
                    sync_params: None,
                })
            }
            BlockchainProviderConfig::Electrum { url } => {
                let config = electrum_client::ConfigBuilder::new()
                    .retry(3)
                    .timeout(Some(60))
                    .build();
                let client = electrum_client::Client::from_config(&url, config)
                    .map_err(|e| Error::ElectrumBlockchainFactoryCreationFailed(e.to_string()))?;
                AnyBlockchainFactory::Electrum(Arc::new(ElectrumBlockchain::from(client)))
            }
        })
    }
}

/// A local implementation of an online heritage wallet
///
/// This struct provides a local, file-based implementation of the heritage wallet
/// functionality. It manages wallet state, blockchain synchronization, and transaction
/// operations without requiring a remote service.
///
/// The wallet maintains its own database and can connect to either Bitcoin Core or
/// Electrum servers for blockchain data.
#[derive(Serialize, Deserialize)]
pub struct LocalHeritageWallet {
    /// Unique identifier for this heritage wallet instance
    pub(crate) heritage_wallet_id: String,
    /// BIP32 master key fingerprint, cached to verify the provided Heritage Wallet match when initialized
    fingerprint: Option<Fingerprint>,
    /// The underlying heritage wallet instance (not serialized)
    #[serde(skip, default)]
    heritage_wallet: Option<Arc<Mutex<HeritageWallet<HeritageWalletDatabase>>>>,
    /// Blockchain connection factory (not serialized)
    #[serde(skip, default)]
    blockchain_factory: Option<AnyBlockchainFactory>,
}

impl std::fmt::Debug for LocalHeritageWallet {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("LocalHeritageWallet")
            .field("heritage_wallet_id", &self.heritage_wallet_id)
            .field(
                "heritage_wallet",
                if self.heritage_wallet.is_some() {
                    &"Some(HeritageWallet<...>)"
                } else {
                    &"None"
                },
            )
            .field("blockchain", &self.blockchain_factory)
            .finish()
    }
}

impl LocalHeritageWallet {
    /// Creates a new local heritage wallet instance
    ///
    /// This method initializes a new heritage wallet with a randomly generated ID
    /// and sets up the underlying wallet database. If a backup is provided, it will
    /// be restored during creation.
    ///
    /// # Arguments
    ///
    /// * `db` - Database connection for wallet storage
    /// * `backup` - Optional wallet backup to restore from
    /// * `block_inclusion_objective` - Target number of blocks for transaction confirmation
    ///
    /// # Returns
    ///
    /// Returns a new `LocalHeritageWallet` instance ready for use.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database operations fail
    /// - Backup restoration fails
    /// - Wallet initialization fails
    pub fn create(
        db: &Database,
        backup: Option<HeritageWalletBackup>,
        block_inclusion_objective: BlockInclusionObjective,
    ) -> Result<Self> {
        let heritage_wallet_id = format!("{:032x}", rand::random::<u128>());
        let heritage_wallet = {
            let db = db.clone();
            HeritageWallet::new(HeritageWalletDatabase::create(&heritage_wallet_id, db)?)
        };

        if let Some(backup) = backup {
            heritage_wallet.restore_backup(backup)?;
        }
        let fingerprint = heritage_wallet.fingerprint()?;

        heritage_wallet.set_block_inclusion_objective(block_inclusion_objective)?;

        let heritage_wallet = Some(Arc::new(Mutex::new(heritage_wallet)));
        let local_heritage_wallet = LocalHeritageWallet {
            heritage_wallet_id,
            fingerprint,
            heritage_wallet,
            blockchain_factory: None,
        };

        Ok(local_heritage_wallet)
    }

    /// Deletes the wallet's database table
    ///
    /// This method permanently removes all wallet data from the database.
    /// Use with caution as this operation cannot be undone.
    ///
    /// # Arguments
    ///
    /// * `db` - Mutable reference to the database connection
    ///
    /// # Errors
    ///
    /// Returns a database error if the table deletion fails.
    pub(crate) fn delete(
        &self,
        db: &mut Database,
    ) -> core::result::Result<(), crate::database::errors::DbError> {
        db.drop_table(&self.heritage_wallet_id)?;
        Ok(())
    }

    /// Initializes the heritage wallet component with database access
    ///
    /// This method creates and initializes the underlying heritage wallet instance
    /// using the provided database connection. It must be called before performing
    /// any wallet operations that require access to the heritage wallet.
    ///
    /// The method also performs a fingerprint consistency check to ensure the
    /// loaded wallet matches the expected fingerprint.
    ///
    /// # Arguments
    ///
    /// * `db` - Database connection for the wallet
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if the initialization succeeds.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database operations fail
    /// - Wallet initialization fails
    /// - The loaded wallet's fingerprint doesn't match the cached fingerprint
    pub async fn init_heritage_wallet(&mut self, db: Database) -> Result<()> {
        let id = self.heritage_wallet_id.clone();
        self.heritage_wallet = Some(Arc::new(Mutex::new(HeritageWallet::new(
            blocking_db_operation(db, move |db| HeritageWalletDatabase::get(&id, db)).await?,
        ))));
        let wallet_fg = self.wallet_call(|hw| hw.fingerprint()).await?;
        if wallet_fg != self.fingerprint && self.fingerprint.is_some() {
            return Err(Error::IncoherentLocalWalletFingerprint);
        }
        Ok(())
    }

    /// Executes a function with access to the heritage wallet
    ///
    /// This method provides safe access to the underlying heritage wallet by
    /// spawning a blocking task and acquiring the wallet lock. This ensures
    /// that wallet operations don't block the async runtime.
    ///
    /// # Arguments
    ///
    /// * `f` - Function to execute with wallet access
    ///
    /// # Returns
    ///
    /// Returns the result of the function execution.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The heritage wallet is not initialized
    /// - The function execution fails
    pub(crate) async fn wallet_call<
        R: Send + 'static,
        F: FnOnce(
                &HeritageWallet<HeritageWalletDatabase>,
            ) -> core::result::Result<R, btc_heritage::errors::Error>
            + Send
            + 'static,
    >(
        &self,
        f: F,
    ) -> Result<R> {
        let arc_wallet = self
            .heritage_wallet
            .as_ref()
            .ok_or(Error::UninitializedHeritageWallet)?
            .clone();

        Ok(tokio::task::spawn_blocking(move || {
            let wallet = arc_wallet.lock().unwrap();
            f(&wallet)
        })
        .await
        .unwrap()?)
    }
    /// Synchronizes the cached fingerprint with the heritage wallet's fingerprint
    ///
    /// This method retrieves the current fingerprint from the heritage wallet and
    /// updates the cached fingerprint if they differ.
    ///
    /// # Returns
    ///
    /// Returns `true` if the fingerprint was updated, `false` if it was already
    /// synchronized.
    ///
    /// # Errors
    ///
    /// Returns an error if the heritage wallet is not initialized or if there's
    /// an issue retrieving the fingerprint from the wallet.
    pub(crate) async fn sync_fingerprint(&mut self) -> Result<bool> {
        let wallet_fg = self.wallet_call(|hw| hw.fingerprint()).await?;
        if self.fingerprint != wallet_fg {
            self.fingerprint = wallet_fg;
            Ok(true)
        } else {
            Ok(false)
        }
    }
    /// Initializes the blockchain factory for network operations
    ///
    /// This method sets up the blockchain connection factory that will be used
    /// for synchronization and transaction broadcasting.
    ///
    /// # Arguments
    ///
    /// * `blockchain_factory` - The blockchain factory to use for network operations
    pub fn init_blockchain_factory(&mut self, blockchain_factory: AnyBlockchainFactory) {
        self.blockchain_factory = Some(blockchain_factory);
    }
    /// Gets a reference to the blockchain factory
    ///
    /// # Returns
    ///
    /// Returns a reference to the blockchain factory.
    ///
    /// # Errors
    ///
    /// Returns an error if the blockchain factory is not initialized.
    fn blockchain_factory(&self) -> Result<&AnyBlockchainFactory> {
        self.blockchain_factory
            .as_ref()
            .ok_or(Error::UninitializedBlockchainFactory)
    }
}

impl super::OnlineWallet for LocalHeritageWallet {
    async fn backup_descriptors(&self) -> Result<HeritageWalletBackup> {
        self.wallet_call(|wallet| wallet.generate_backup()).await
    }

    async fn get_address(&self) -> Result<WalletAddress> {
        self.wallet_call(|wallet| wallet.get_new_address()).await
    }

    async fn list_addresses(&self) -> Result<Vec<WalletAddress>> {
        self.wallet_call(|wallet| wallet.list_wallet_addresses())
            .await
    }

    async fn list_transactions(&self) -> Result<Vec<TransactionSummary>> {
        self.wallet_call(|wallet| Ok(wallet.database().list_transaction_summaries()?))
            .await
    }

    async fn list_heritage_utxos(&self) -> Result<Vec<heritage_service_api_client::HeritageUtxo>> {
        self.wallet_call(|wallet| Ok(wallet.database().list_utxos()?))
            .await
    }

    async fn list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>> {
        let (used_account_xpubs, unused_account_xpubs) = self
            .wallet_call(|wallet| {
                Ok((
                    wallet.list_used_account_xpubs()?,
                    wallet.list_unused_account_xpubs()?,
                ))
            })
            .await?;

        Ok(used_account_xpubs
            .into_iter()
            .map(|ad| AccountXPubWithStatus::Used(ad))
            .chain(
                unused_account_xpubs
                    .into_iter()
                    .map(|ad| AccountXPubWithStatus::Unused(ad)),
            )
            .collect::<Vec<_>>())
    }

    async fn feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()> {
        let fingerprint = self
            .wallet_call(move |wallet| {
                wallet.append_account_xpubs(account_xpubs)?;
                wallet.fingerprint()
            })
            .await?;

        if self.fingerprint.is_none() {
            self.fingerprint = fingerprint;
        }
        Ok(())
    }

    async fn list_subwallet_configs(&self) -> Result<Vec<SubwalletConfigMeta>> {
        self.wallet_call(move |wallet| {
            let mut obsolete_subwallet_configs =
                wallet.database().list_obsolete_subwallet_configs()?;
            if let Some(swc) = wallet
                .database()
                .get_subwallet_config(SubwalletConfigId::Current)?
            {
                obsolete_subwallet_configs.push(swc);
            }
            obsolete_subwallet_configs.reverse();
            Ok(obsolete_subwallet_configs
                .into_iter()
                .map(SubwalletConfigMeta::from)
                .collect())
        })
        .await
    }

    async fn list_heritage_configs(&self) -> Result<Vec<HeritageConfig>> {
        self.wallet_call(|wallet| {
            let mut obsolete_heritage_configs = wallet.list_obsolete_heritage_configs()?;
            if let Some(hc) = wallet.get_current_heritage_config()? {
                obsolete_heritage_configs.push(hc);
            }
            obsolete_heritage_configs.reverse();
            Ok(obsolete_heritage_configs)
        })
        .await
    }

    async fn set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig> {
        self.wallet_call(move |wallet| {
            wallet.update_heritage_config(new_hc.clone())?;
            Ok(new_hc)
        })
        .await
    }

    async fn set_block_inclusion_objective(
        &mut self,
        bio: BlockInclusionObjective,
    ) -> Result<super::WalletStatus> {
        self.wallet_call(move |wallet| wallet.set_block_inclusion_objective(bio))
            .await?;
        self.get_wallet_status().await
    }

    async fn sync(&mut self) -> Result<()> {
        let bcf = self.blockchain_factory()?.clone();
        self.wallet_call(move |wallet| match bcf {
            AnyBlockchainFactory::Bitcoin(bcf) => wallet.sync(&bcf),
            AnyBlockchainFactory::Electrum(bcf) => wallet.sync(&bcf),
        })
        .await?;

        Ok(())
    }

    async fn get_wallet_status(&self) -> Result<super::WalletStatus> {
        self.wallet_call(|wallet| {
            let last_fee_rate = wallet.database().get_fee_rate()?;
            Ok(super::WalletStatus {
                fingerprint: wallet.fingerprint()?,
                balance: wallet.get_balance()?,
                last_sync_ts: wallet
                    .get_sync_time()?
                    .map(|bt| bt.timestamp)
                    .unwrap_or_default(),
                block_inclusion_objective: wallet.get_block_inclusion_objective()?,
                last_fee_rate,
            })
        })
        .await
    }

    async fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        let NewTx {
            spending_config,
            fee_policy,
            utxo_selection,
            disable_rbf,
        } = new_tx;
        let spending_config = match spending_config {
            heritage_service_api_client::NewTxSpendingConfig::Recipients(recipients) => {
                SpendingConfig::try_from(
                    recipients
                        .into_iter()
                        .map(|r| (r.address, Amount::from_sat(r.amount)))
                        .collect::<Vec<_>>(),
                )?
            }
            heritage_service_api_client::NewTxSpendingConfig::DrainTo(NewTxDrainTo {
                drain_to,
            }) => SpendingConfig::DrainTo(btc_heritage::utils::string_to_address(&drain_to)?),
        };
        let create_psbt_options = CreatePsbtOptions {
            fee_policy: fee_policy.map(|fp| fp.into()),
            utxo_selection: utxo_selection.map(|us| us.into()).unwrap_or_default(),
            disable_rbf: disable_rbf.unwrap_or_default(),
            ..Default::default()
        };
        self.wallet_call(move |wallet| {
            wallet.create_owner_psbt(spending_config, create_psbt_options)
        })
        .await
    }
}

impl Broadcaster for LocalHeritageWallet {
    /// Broadcasts a signed transaction to the network
    ///
    /// This method extracts the final transaction from the PSBT and broadcasts it
    /// using the configured blockchain provider (Bitcoin Core or Electrum).
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
    /// - Transaction extraction from PSBT fails
    /// - Blockchain factory is not initialized
    /// - Network broadcast fails
    async fn broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid> {
        let tx = btc_heritage::utils::extract_tx(psbt)?;
        let bcf = self.blockchain_factory()?.clone();
        tokio::task::spawn_blocking(move || match bcf {
            AnyBlockchainFactory::Bitcoin(bcf) => {
                let rpc_client = Client::new(&bcf.url, bcf.auth.clone().into())
                    .map_err(|e| Error::generic(e))?;
                Ok(rpc_client
                    .send_raw_transaction(&tx)
                    .map_err(|e| Error::generic(e))?)
            }
            AnyBlockchainFactory::Electrum(bcf) => Ok(bcf
                .transaction_broadcast_raw(
                    btc_heritage::bitcoin::consensus::encode::serialize(&tx).as_ref(),
                )
                .map_err(|e| Error::generic(e))?),
        })
        .await
        .unwrap()
    }
}
impl BoundFingerprint for LocalHeritageWallet {
    /// Returns the BIP32 master key fingerprint
    ///
    /// # Returns
    ///
    /// Returns the cached fingerprint of the wallet's master key.
    ///
    /// # Errors
    ///
    /// Returns an error if the fingerprint is not available (online wallet never had account pub keys).
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self
            .fingerprint
            .ok_or(Error::OnlineWalletFingerprintNotPresent)?)
    }
}
