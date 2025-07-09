use std::{
    fmt::Debug,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{
    database::{dbitem::impl_db_single_item, HeritageWalletDatabase},
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

/// Authentication configuration for Bitcoin Core
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuthConfig {
    /// Cookie file authentication
    Cookie {
        /// Path to the cookie file
        file: Arc<str>,
    },
    /// Username/password authentication
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BlockchainProviderConfig {
    BitcoinCore { url: Arc<str>, auth: AuthConfig },
    Electrum { url: Arc<str> },
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

#[derive(Clone)]
pub enum AnyBlockchainFactory {
    Bitcoin(RpcBlockchainFactory),
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

#[derive(Serialize, Deserialize)]
pub struct LocalHeritageWallet {
    pub(crate) heritage_wallet_id: String,
    fingerprint: Option<Fingerprint>,
    #[serde(skip, default)]
    heritage_wallet: Option<Arc<Mutex<HeritageWallet<HeritageWalletDatabase>>>>,
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

    pub(crate) fn delete(
        &self,
        db: &mut Database,
    ) -> core::result::Result<(), crate::database::errors::DbError> {
        db.drop_table(&self.heritage_wallet_id)?;
        Ok(())
    }

    pub fn init_heritage_wallet(&mut self, db: Database) -> Result<()> {
        self.heritage_wallet = Some(Arc::new(Mutex::new(HeritageWallet::new(
            HeritageWalletDatabase::get(&self.heritage_wallet_id, db)?,
        ))));
        Ok(())
    }

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

    pub fn init_blockchain_factory(&mut self, blockchain_factory: AnyBlockchainFactory) {
        self.blockchain_factory = Some(blockchain_factory);
    }
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
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self
            .fingerprint
            .ok_or(Error::OnlineWalletFingerprintNotPresent)?)
    }
}
