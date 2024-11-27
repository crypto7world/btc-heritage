use std::{fmt::Debug, sync::Arc};

use crate::{
    database::HeritageWalletDatabase,
    errors::{Error, Result},
    BoundFingerprint, Broadcaster, Database,
};
use btc_heritage::{
    bdk_types::{ElectrumBlockchain, RpcBlockchainFactory},
    bitcoin::{bip32::Fingerprint, secp256k1::rand, Txid},
    bitcoincore_rpc::{Client, RpcApi},
    database::HeritageDatabase,
    electrum_client::ElectrumApi,
    heritage_wallet::{CreatePsbtOptions, TransactionSummary, WalletAddress},
    AccountXPub, Amount, BlockInclusionObjective, HeritageConfig, HeritageWallet,
    HeritageWalletBackup, PartiallySignedTransaction, SpendingConfig,
};
use heritage_service_api_client::{AccountXPubWithStatus, NewTx, NewTxDrainTo};

use serde::{Deserialize, Serialize};

use super::OnlineWallet;

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
                Self::Bitcoin(_) => "Bitcoin(...)",
                Self::Electrum(_) => "Electrum(...)",
            }
        )
    }
}

#[derive(Serialize, Deserialize)]
pub struct LocalHeritageWallet {
    heritage_wallet_id: String,
    fingerprint: Option<Fingerprint>,
    #[serde(skip, default)]
    heritage_wallet: Option<HeritageWallet<HeritageWalletDatabase>>,
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
        block_inclusion_objective: u16,
    ) -> Result<Self> {
        let heritage_wallet_id = format!("{:032x}", rand::random::<u128>());
        let heritage_wallet = HeritageWallet::new(HeritageWalletDatabase::create(
            heritage_wallet_id.clone(),
            db,
        )?);
        if let Some(backup) = backup {
            heritage_wallet.restore_backup(backup)?;
        }
        let fingerprint = heritage_wallet.fingerprint()?;
        let heritage_wallet = Some(heritage_wallet);
        let mut local_heritage_wallet = LocalHeritageWallet {
            heritage_wallet_id,
            fingerprint,
            heritage_wallet,
            blockchain_factory: None,
        };
        local_heritage_wallet.set_block_inclusion_objective(block_inclusion_objective)?;
        Ok(local_heritage_wallet)
    }

    pub(crate) fn delete(
        &self,
        db: &mut Database,
    ) -> core::result::Result<(), crate::database::errors::DbError> {
        db.drop_table(&self.heritage_wallet_id)?;
        Ok(())
    }

    pub fn init_heritage_wallet(&mut self, db: &Database) -> Result<()> {
        self.heritage_wallet = Some(HeritageWallet::new(HeritageWalletDatabase::get(
            self.heritage_wallet_id.clone(),
            db,
        )?));
        Ok(())
    }
    pub(crate) fn heritage_wallet(&self) -> &HeritageWallet<HeritageWalletDatabase> {
        self.heritage_wallet
            .as_ref()
            .expect("heritage wallet should have been initialized")
    }

    pub fn init_blockchain_factory(
        &mut self,
        blockchain_factory: AnyBlockchainFactory,
    ) -> Result<()> {
        self.blockchain_factory = Some(blockchain_factory);
        Ok(())
    }
    fn blockchain_factory(&self) -> &AnyBlockchainFactory {
        self.blockchain_factory
            .as_ref()
            .expect("blockchain factory should have been initialized")
    }
}

impl super::OnlineWallet for LocalHeritageWallet {
    fn backup_descriptors(&self) -> Result<HeritageWalletBackup> {
        Ok(self.heritage_wallet().generate_backup()?)
    }

    fn get_address(&self) -> Result<String> {
        Ok(self.heritage_wallet().get_new_address()?.to_string())
    }

    fn list_addresses(&self) -> Result<Vec<WalletAddress>> {
        Ok(self.heritage_wallet().list_wallet_addresses()?)
    }

    fn list_transactions(&self) -> Result<Vec<TransactionSummary>> {
        Ok(self
            .heritage_wallet()
            .database()
            .list_transaction_summaries()?)
    }

    fn list_heritage_utxos(&self) -> Result<Vec<heritage_service_api_client::HeritageUtxo>> {
        Ok(self.heritage_wallet().database().list_utxos()?)
    }

    fn list_account_xpubs(&self) -> Result<Vec<AccountXPubWithStatus>> {
        let wallet = self.heritage_wallet();
        let used_account_xpubs = wallet.list_used_account_xpubs()?;
        let unused_account_xpubs = wallet.list_unused_account_xpubs()?;
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

    fn feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()> {
        self.heritage_wallet().append_account_xpubs(account_xpubs)?;
        if self.fingerprint.is_none() {
            self.fingerprint = self.heritage_wallet().fingerprint()?;
        }
        Ok(())
    }

    fn list_heritage_configs(&self) -> Result<Vec<HeritageConfig>> {
        let wallet = self.heritage_wallet();
        let mut obsolete_heritage_configs = wallet.list_obsolete_heritage_configs()?;
        if let Some(hc) = wallet.get_current_heritage_config()? {
            obsolete_heritage_configs.push(hc);
        }
        obsolete_heritage_configs.reverse();
        Ok(obsolete_heritage_configs)
    }

    fn set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig> {
        self.heritage_wallet()
            .update_heritage_config(new_hc.clone())?;
        Ok(new_hc)
    }

    fn set_block_inclusion_objective(&mut self, bio: u16) -> Result<super::WalletStatus> {
        self.heritage_wallet()
            .set_block_inclusion_objective(BlockInclusionObjective::from(bio))?;
        self.get_wallet_status()
    }

    fn sync(&mut self) -> Result<()> {
        let wallet = self.heritage_wallet();
        match self.blockchain_factory() {
            AnyBlockchainFactory::Bitcoin(bcf) => wallet.sync(bcf)?,
            AnyBlockchainFactory::Electrum(bcf) => wallet.sync(bcf)?,
        }
        Ok(())
    }

    fn get_wallet_status(&self) -> Result<super::WalletStatus> {
        let wallet = self.heritage_wallet();
        Ok(super::WalletStatus {
            fingerprint: wallet.fingerprint()?,
            balance: wallet.get_balance()?,
            last_sync_ts: wallet
                .get_sync_time()?
                .map(|bt| bt.timestamp)
                .unwrap_or_default(),
            block_inclusion_objective: wallet.get_block_inclusion_objective()?,
            last_fee_rate: wallet.database().get_fee_rate()?,
        })
    }

    fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        let wallet = self.heritage_wallet();
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
        Ok(wallet.create_owner_psbt(spending_config, create_psbt_options)?)
    }
}

impl Broadcaster for LocalHeritageWallet {
    fn broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid> {
        let tx = btc_heritage::utils::extract_tx(psbt)?;
        match self.blockchain_factory() {
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
        }
    }
}
impl BoundFingerprint for LocalHeritageWallet {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self
            .fingerprint
            .ok_or(Error::OnlineWalletFingerprintNotPresent)?)
    }
}
