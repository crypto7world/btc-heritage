pub mod backup;
#[cfg(any(feature = "online", test))]
pub mod online;
mod types;

use core::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    account_xpub::AccountXPub,
    bitcoin::{
        absolute::LockTime,
        bip32::Fingerprint,
        psbt::{Input, Output, Psbt},
        Address, Amount, FeeRate, OutPoint, Script, Sequence, TxOut, Weight,
    },
    database::{
        PartitionableDatabase, SubdatabaseId, TransacHeritageDatabase, TransacHeritageOperation,
    },
    errors::{DatabaseError, Error, Result},
    heritage_config::{HeritageConfig, HeritageExplorer, HeritageExplorerTrait},
    miniscript::{Miniscript, Tap},
    subwallet_config::SubwalletConfig,
    utils::bitcoin_network_from_env,
    HeirConfig,
};

use backup::{HeritageWalletBackup, SubwalletDescriptorBackup};
use bdk::{
    database::Database,
    wallet::{AddressIndex, AddressInfo, IsDust},
    BlockTime, FeeRate as BdkFeeRate, KeychainKind, LocalUtxo, Wallet,
};

pub use types::*;

#[derive(Debug, Clone)]
enum Spender {
    Owner,
    Heir(HeirConfig),
}

pub struct HeritageWallet<D: TransacHeritageDatabase> {
    database: RefCell<D>,
}

impl<D: TransacHeritageDatabase> HeritageWallet<D> {
    pub fn new(database: D) -> Self {
        log::debug!("HeritageWallet::new");
        Self {
            database: RefCell::new(database),
        }
    }

    pub fn generate_backup(&self) -> Result<HeritageWalletBackup> {
        log::debug!("HeritageWallet::generate_backup");
        Ok(HeritageWalletBackup(
            self.database
                .borrow()
                .list_obsolete_subwallet_configs()?
                .into_iter()
                .chain(
                    self.database
                        .borrow()
                        .get_subwallet_config(SubwalletConfigId::Current)?,
                )
                .map(|swc| {
                    let sw = self.get_subwallet(&swc)?;
                    let last_external_index = sw
                        .database()
                        .get_last_index(KeychainKind::External)
                        .map_err(|e| DatabaseError::Generic(e.to_string()))?;
                    let last_change_index = sw
                        .database()
                        .get_last_index(KeychainKind::Internal)
                        .map_err(|e| DatabaseError::Generic(e.to_string()))?;

                    Ok(SubwalletDescriptorBackup {
                        external_descriptor: swc.ext_descriptor().clone(),
                        change_descriptor: swc.change_descriptor().clone(),
                        first_use_ts: swc.subwallet_firstuse_time(),
                        last_external_index,
                        last_change_index,
                    })
                })
                .collect::<Result<_>>()?,
        ))
    }

    pub fn restore_backup(&self, backup: HeritageWalletBackup) -> Result<()> {
        log::debug!("HeritageWallet::restore_backup - backup={backup:?}");
        if backup.0.len() == 0 {
            return Ok(());
        }

        log::info!(
            "HeritageWallet::restore_backup - \
        Trying to restore backup with {} SubwalletDescriptorBackup(s)",
            backup.0.len()
        );
        // See if we can get all the configs
        let mut swc_and_backups = backup
            .into_iter()
            .map(|swc_backup| Ok((SubwalletConfig::try_from(&swc_backup)?, swc_backup)))
            .collect::<Result<Vec<_>>>()?;

        log::info!("HeritageWallet::restore_backup - All SubwalletConfig(s) created");
        // Ensure they are sorted by ID
        swc_and_backups.sort_by_key(|(swc, _)| swc.subwallet_id());

        // Try to commit everything in one transaction
        let last_id = swc_and_backups
            .last()
            .expect("At least one")
            .0
            .subwallet_id();
        log::debug!("HeritageWallet::restore_backup - last_id={last_id}");
        let mut transaction = self.database.borrow().begin_transac();
        for (swc, _) in swc_and_backups.iter() {
            let swc_id = swc.subwallet_id();
            let swc_id = if swc_id == last_id {
                SubwalletConfigId::Current
            } else {
                SubwalletConfigId::Id(swc_id)
            };
            log::debug!(
                "HeritageWallet::restore_backup - \
            swc_id={swc_id:?} swc={swc:?}"
            );
            transaction.put_subwallet_config(swc_id, swc)?;
        }
        self.database.borrow_mut().commit_transac(transaction)?;
        log::info!("HeritageWallet::restore_backup - All SubwalletConfig(s) written to DB");

        for (swc, swc_backup) in swc_and_backups.into_iter() {
            if swc_backup.last_external_index.is_some() || swc_backup.last_change_index.is_some() {
                let sw = self.get_subwallet(&swc)?;
                if let Some(last_external_index) = swc_backup.last_external_index {
                    log::info!(
                        "HeritageWallet::restore_backup - \
                    SubwalletConfigId({}) reset external index {last_external_index}",
                        swc.subwallet_id()
                    );
                    sw.get_address(AddressIndex::Reset(last_external_index))
                        .map_err(|e| Error::FailedToResetAddressIndex(e.to_string()))?;
                }
                if let Some(last_change_index) = swc_backup.last_change_index {
                    log::info!(
                        "HeritageWallet::restore_backup - \
                    SubwalletConfigId({}) reset change index {last_change_index}",
                        swc.subwallet_id()
                    );
                    sw.get_internal_address(AddressIndex::Reset(last_change_index))
                        .map_err(|e| Error::FailedToResetAddressIndex(e.to_string()))?;
                }
            }
        }
        log::info!("HeritageWallet::restore_backup - Done");
        Ok(())
    }

    pub fn list_wallet_addresses(&self) -> Result<Vec<WalletAddress>> {
        log::debug!("HeritageWallet::list_wallet_addresses");
        let Some(fingerprint) = self.fingerprint()? else {
            // No fingerprint means no AccountXPub, so no address either
            return Ok(vec![]);
        };

        let intermediate_results = self
            .database
            .borrow()
            .list_obsolete_subwallet_configs()?
            .into_iter()
            .chain(
                self.database
                    .borrow()
                    .get_subwallet_config(SubwalletConfigId::Current)?,
            )
            // Map each subwallet config to a WalletAddress iterator
            .map(|swc| {
                // Retrieve the derivation path of the account xpub
                let axpub_dp = swc
                    .account_xpub()
                    .descriptor_public_key()
                    .full_derivation_path()
                    .expect("DerivationPath is present for an Account Xpub");
                let mut axpub_dpi = axpub_dp.normal_children();

                // Construct the external and change DerivationPath
                let (ext_dp, change_dp) = (axpub_dpi.next().unwrap(), axpub_dpi.next().unwrap());

                // Open the Subwallet DB
                let sw = self.get_subwallet(&swc)?;

                // Retrieve the last external index
                let last_external_index = sw
                    .database()
                    .get_last_index(KeychainKind::External)
                    .map_err(|e| DatabaseError::Generic(e.to_string()))?;

                // Retrieve the last change index
                let last_change_index = sw
                    .database()
                    .get_last_index(KeychainKind::Internal)
                    .map_err(|e| DatabaseError::Generic(e.to_string()))?;

                // For each (index, keychain, derivation_path)
                let wallet_addresses = [
                    (last_change_index, KeychainKind::Internal, change_dp),
                    (last_external_index, KeychainKind::External, ext_dp),
                ]
                .into_iter()
                // Filtermap, if last index is present, find all address up to that index
                // Else, do nothing
                .filter_map(|(last_index, kc, dp)| {
                    last_index.map(|last_index| {
                        let wallet_addresses = sw
                            .database()
                            .iter_script_pubkeys(Some(kc))
                            .map_err(|e| DatabaseError::Generic(e.to_string()))?
                            .into_iter()
                            .zip(dp.normal_children())
                            .take((last_index + 1) as usize)
                            .map(|(sb, dp)| WalletAddress {
                                origin: (fingerprint, dp),
                                address: Address::from_script(
                                    sb.as_script(),
                                    *bitcoin_network_from_env(),
                                )
                                .expect(
                                    "script should always be valid from the \
                                correct network inside the DB",
                                ),
                            })
                            .collect::<Vec<_>>();
                        Ok(wallet_addresses)
                    })
                })
                .collect::<Result<Vec<_>>>()?;
                Ok(wallet_addresses)
            })
            // Flatten to get a single WalletAddress iterator
            .collect::<Result<Vec<_>>>()?;

        // At this point we got a Vec<Vec<Vec<WalletAddress>>>
        // We flatten and reverse it
        // Reverse so that the result vec starts with external addresses of the current config,
        // then change addresses of the current config, then external of the previous configs, etc...
        Ok(intermediate_results
            .into_iter()
            .flatten()
            .flatten()
            .rev()
            .collect())
    }

    /// Return an immutable reference to the internal database
    pub fn database(&self) -> impl core::ops::Deref<Target = D> + '_ {
        self.database.borrow()
    }

    pub fn list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageWallet::list_used_account_xpubs");
        let res = self.database.borrow().list_used_account_xpubs()?;
        log::debug!("HeritageWallet::list_used_account_xpubs - res={res:?}");
        Ok(res)
    }

    pub fn list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageWallet::list_unused_account_xpubs");
        let res = self.database.borrow().list_unused_account_xpubs()?;
        log::debug!("HeritageWallet::list_unused_account_xpubs - res={res:?}");
        Ok(res)
    }

    /// Returns the fingerprint of the Heritage Wallet master key
    /// if the wallet already has Account Xpubs
    /// Else return None
    pub fn fingerprint(&self) -> Result<Option<Fingerprint>> {
        log::debug!("HeritageWallet::fingerprint");
        let res = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
            .map(|swc| {
                swc.account_xpub()
                    .descriptor_public_key()
                    .master_fingerprint()
            });

        let res = if res.is_none() {
            log::debug!(
                "HeritageWallet::fingerprint - No Current SubwalletConfig, \
            trying to find fingerprint on an Unused Account XPub"
            );
            self.database
                .borrow()
                .get_unused_account_xpub()?
                .map(|axpub| axpub.descriptor_public_key().master_fingerprint())
        } else {
            res
        };
        log::debug!("HeritageWallet::fingerprint - res={res:?}");
        Ok(res)
    }

    pub fn get_sync_time(&self) -> Result<Option<BlockTime>> {
        if let Some(current_subwalletconfig) = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
        {
            if let Some(sync_time) = self
                .get_subwallet(&current_subwalletconfig)?
                .database()
                .get_sync_time()
                .map_err(|e| DatabaseError::Generic(e.to_string()))?
            {
                return Ok(Some(sync_time.block_time));
            }
            let obsolete_subwalletconfigs =
                self.database.borrow().list_obsolete_subwallet_configs()?;
            for obsolete_subwalletconfig in obsolete_subwalletconfigs {
                if let Some(sync_time) = self
                    .get_subwallet(&obsolete_subwalletconfig)?
                    .database()
                    .get_sync_time()
                    .map_err(|e| DatabaseError::Generic(e.to_string()))?
                {
                    return Ok(Some(sync_time.block_time));
                }
            }
        }
        Ok(None)
    }

    /// Verify if a ScriptPubKey belong to the wallet of current [HeritageConfig] of this
    /// [HeritageWallet].
    ///
    /// # Errors
    ///
    /// This function will return an error if there are problems with the database.
    pub fn is_mine_and_current(&self, script: &Script) -> Result<bool> {
        match self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
        {
            Some(subwalletconfig) => Ok(self
                .get_subwallet(&subwalletconfig)?
                .is_mine(script)
                .map_err(|e| DatabaseError::Generic(e.to_string()))?),
            None => Ok(false),
        }
    }

    /// Verify if a ScriptPubKey belong to one of the wallets resulting from all the [HeritageConfig]
    /// of this [HeritageWallet].
    ///
    /// Note that if there is no current [HeritageConfig] the function will not look further
    /// as the database integrity should ensure that the only situation where there is
    /// no "current" [HeritageConfig] is if there is no [HeritageConfig] at all.
    ///
    /// # Errors
    ///
    /// This function will return an error if there are problems with the database.
    pub fn is_mine(&self, script: &Script) -> Result<bool> {
        if let Some(current_subwalletconfig) = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
        {
            if self
                .get_subwallet(&current_subwalletconfig)?
                .is_mine(script)
                .map_err(|e| DatabaseError::Generic(e.to_string()))?
            {
                return Ok(true);
            }
            let mut obsolete_subwalletconfigs =
                self.database.borrow().list_obsolete_subwallet_configs()?;
            obsolete_subwalletconfigs.reverse();
            for obsolete_subwalletconfig in obsolete_subwalletconfigs {
                if self
                    .get_subwallet(&obsolete_subwalletconfig)?
                    .is_mine(script)
                    .map_err(|e| DatabaseError::Generic(e.to_string()))?
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Try to add new [AccountXPub] to this [HeritageWallet].
    ///
    /// Note that the ID of an [AccountXPub] is determined by the 3rd node of the derivation path,
    /// usually called "account", irrespective of the fingerprint. It means that if you try to add
    /// an [AccountXPub] with the same ID (3rd node of the derivation path) than an existing one but
    /// another fingerprint, the API will consider those [AccountXPub] are the same. If the existing
    /// [AccountXPub] has already been used, the new one will be discarded, else the new one will replace
    /// the existing one.
    ///
    /// While this does not prevent the usage of [AccountXPub] of mixed origin (different
    /// fingerprints) as long as the 3rd node of the derivation paths are all unique, you
    /// are strongly discouraged to do this. A different fingerprint means a different root
    /// master key and, ultimatly, a different wallet.
    ///
    /// # Errors
    /// This function will return an error if there are problems with the database.
    pub fn append_account_xpubs(
        &self,
        account_xpubs: impl IntoIterator<Item = AccountXPub>,
    ) -> Result<()> {
        log::debug!("HeritageWallet::append_account_xpubs");
        let account_xpubs = account_xpubs.into_iter().collect::<Vec<_>>();
        if let Some(fingerprint) = self.fingerprint()? {
            if account_xpubs
                .iter()
                .any(|axpub| axpub.descriptor_public_key().master_fingerprint() != fingerprint)
            {
                log::error!("Cannot add Account Xpubs that does not have the same Fingerprint as the Heritage wallet");
                return Err(Error::InvalidAccountXPub);
            }
        } else {
            let reference = account_xpubs
                .first()
                .map(|axpub| axpub.descriptor_public_key().master_fingerprint());
            if account_xpubs.iter().any(|axpub| {
                axpub.descriptor_public_key().master_fingerprint() != reference.unwrap()
            }) {
                log::error!("Cannot add Account Xpubs that does not all have the same Fingerprint");
                return Err(Error::InvalidAccountXPub);
            }
        }
        log::debug!("HeritageWallet::append_account_xpubs - account_xpubs={account_xpubs:?}");
        self.database
            .borrow_mut()
            .add_unused_account_xpubs(&account_xpubs)
            .map_err(Into::into)
    }

    pub fn update_heritage_config(&self, new_heritage_config: HeritageConfig) -> Result<()> {
        log::debug!(
            "HeritageWallet::update_heritage_config - new_heritage_config={new_heritage_config:?}"
        );
        log::info!("HeritageWallet::update_heritage_config - Called for an HeritageConfig update");

        // If we previously saw this HeritageConfig, bail
        if self
            .list_obsolete_heritage_configs()?
            .into_iter()
            .any(|previous| previous == new_heritage_config)
        {
            log::error!("Cannot re-use an old HeritageConfig");
            return Err(Error::HeritageConfigAlreadyUsed);
        }

        // Get the current subwallet_config if any
        let Some(current_subwallet_config) = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
        else {
            log::debug!("HeritageWallet::update_heritage_config - No Current SubwalletConfig");
            return self.create_new_subwallet_config(new_heritage_config, None);
        };
        log::debug!(
            "HeritageWallet::update_heritage_config - current_subwallet_config={current_subwallet_config:?}"
        );
        let current_heritage_config = current_subwallet_config.heritage_config();
        log::debug!(
            "HeritageWallet::update_heritage_config - current_heritage_config={current_heritage_config:?}"
        );
        // If the new_heritage_config is the same as the existing one, do nothing
        if new_heritage_config == *current_heritage_config {
            log::debug!(
                "HeritageWallet::update_heritage_config - new_heritage_config == current_heritage_config."
            );
            // If same and no contact changed, nothing to do
            log::info!("HeritageWallet::update_heritage_config - Nothing changed in the new HeritageConfig, nothing to do.");
            return Ok(());
        }

        // Verify if the current_subwallet_config has been used or not
        if current_subwallet_config.subwallet_firstuse_time().is_none() {
            // if it was never used, just override it
            log::debug!(
                "HeritageWallet::update_heritage_config - current_subwallet_config.subwallet_firstuse_time().is_none()"
            );
            let new_subwallet_config = SubwalletConfig::new(
                current_subwallet_config.account_xpub().clone(),
                new_heritage_config,
            );
            let old_subwallet_config = current_subwallet_config;
            log::info!("HeritageWallet::update_heritage_config - Overriding previously unused SubwalletConfig");
            log::debug!(
                "HeritageWallet::update_heritage_config - new_subwallet_config={new_subwallet_config:?}"
            );
            self.database
                .borrow_mut()
                .safe_update_current_subwallet_config(
                    &new_subwallet_config,
                    Some(&old_subwallet_config),
                )
                .map_err(Into::into)
        } else {
            // If it has been used, call the full update procedue
            log::debug!(
                "HeritageWallet::update_heritage_config - current_subwallet_config.subwallet_firstuse_time().is_some()"
            );
            self.create_new_subwallet_config(new_heritage_config, Some(current_subwallet_config))
        }
    }

    pub fn get_current_heritage_config(&self) -> Result<Option<HeritageConfig>> {
        log::debug!("HeritageWallet::get_current_heritage_config");
        // Get the current subwallet_config
        // return the HeritageConfig
        let res = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)
            .map(|subwallet_config| subwallet_config.map(|s| s.into_parts().1))?;
        log::debug!("HeritageWallet::get_current_heritage_config - res={res:?}");
        Ok(res)
    }

    pub fn list_obsolete_heritage_configs(&self) -> Result<Vec<HeritageConfig>> {
        log::debug!("HeritageWallet::list_obsolete_heritage_configs");
        // Get the obsolete subwallet_configs
        // return the HeritageConfigs
        let res = self
            .database
            .borrow()
            .list_obsolete_subwallet_configs()?
            .into_iter()
            .map(|subwallet_config| subwallet_config.into_parts().1)
            .collect();
        log::debug!("HeritageWallet::list_obsolete_heritage_configs - res={res:?}");
        Ok(res)
    }

    pub fn get_balance(&self) -> Result<HeritageWalletBalance> {
        log::debug!("HeritageWallet::get_balance");
        let res = self.database.borrow().get_balance()?.unwrap_or_default();
        log::debug!("HeritageWallet::get_balance - res={res:?}");
        Ok(res)
    }

    pub fn get_new_address(&self) -> Result<Address> {
        log::info!("HeritageWallet::get_new_address - Called for a new Bitcoin address");
        let address = self
            .internal_get_new_address(KeychainKind::External)?
            .address;
        log::info!("HeritageWallet::get_new_address - address={address}");
        Ok(address)
    }

    pub fn get_block_inclusion_objective(&self) -> Result<BlockInclusionObjective> {
        Ok(self
            .database
            .borrow()
            .get_block_inclusion_objective()?
            .unwrap_or_default())
    }

    pub fn set_block_inclusion_objective(&self, new_bio: BlockInclusionObjective) -> Result<()> {
        self.database
            .borrow_mut()
            .set_block_inclusion_objective(new_bio)
            .map_err(|e| DatabaseError::Generic(e.to_string()).into())
    }

    pub fn create_owner_psbt(
        &self,
        spending_config: SpendingConfig,
    ) -> Result<(Psbt, TransactionSummary)> {
        log::debug!("HeritageWallet::create_owner_psbt - spending_config={spending_config:?}");
        self.create_psbt(Spender::Owner, spending_config, None)
    }

    pub fn create_heir_psbt(
        &self,
        heir_config: HeirConfig,
        spending_config: SpendingConfig,
        assume_blocktime: Option<BlockTime>,
    ) -> Result<(Psbt, TransactionSummary)> {
        log::debug!("HeritageWallet::create_heir_psbt - heir_config={heir_config:?} spending_config={spending_config:?}");
        self.create_psbt(
            Spender::Heir(heir_config),
            spending_config,
            assume_blocktime,
        )
    }

    fn create_psbt(
        &self,
        spender: Spender,
        spending_config: SpendingConfig,
        assume_blocktime: Option<BlockTime>,
    ) -> Result<(Psbt, TransactionSummary)> {
        log::debug!(
            "HeritageWallet::create_psbt - spender={spender:?} spending_config={spending_config:?} assume_blocktime={assume_blocktime:?}"
        );
        // When the owner is spending, we want to drain all the obsolete wallets.
        // The current wallet is used only if draining old wallets is not enough to fullfil the requested paiement
        // In the likely case where draining old wallets is enough to fullfil the requested paiement, the remainder is send as change in the current wallet

        // When an heir is spending, we want to take eligible UTXO from the old wallets, taking the closest to loss of exclusivity first
        // We cannot overshot, as we would not now what to do with the change (assuming that, if the heir is spending, to owner is likely dead or otherwise impaired)
        // For now, the wisest course of action is probably to only allow Heir to drain wallets, meaning they receive every eligible UTXO

        // Walk over every spender-eligible UTXO in all subwallets, ordered by loss of exclusivity
        // Eligible UTXO = UTXO that the spender can use given the current block_height (owner is typically always eligible, heirs must wait certain dates)
        // Loss of exlusivity = block_height at which another new spender will be able to spend, for the owner it is typically the first Heir, for an Heir the next-in-line Heir

        // As soon as enough UTXO has been gathered to satisfy the spending_config, stop walking over the subwallets
        // Each subwallet will yield a PSBT, we just have to merge them to produce the expected PSBT of this function

        let heir_spending = if let Spender::Heir(_) = spender {
            true
        } else {
            false
        };

        // For now, we only accept SpendingConfig::DrainTo if it is an Heir spender
        if heir_spending {
            let SpendingConfig::DrainTo(_) = spending_config else {
                log::error!("An Heir can only use SpendingConfig::DrainTo(...)");
                return Err(Error::InvalidSpendingConfigForHeir);
            };
        };

        // We do this now so if it fails we don't bother to go further
        let current_subwallet_config = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
            .ok_or(Error::MissingCurrentSubwalletConfig)?;
        log::debug!(
            "HeritageWallet::create_psbt - current_subwallet_config={current_subwallet_config:?}"
        );
        let current_subwallet = self.get_subwallet(&current_subwallet_config)?;

        // Gather all the UTXO of the obsolete wallet configs
        log::debug!("HeritageWallet::create_psbt - Listing obsolete subwallet_configs");
        let obsolete_subwallet_configs =
            self.database.borrow().list_obsolete_subwallet_configs()?;

        // Here we compute what will be the "present" for this PSBT creation
        // If we got it as a paramter, just use it
        // Else we create a fake BlockTime with the last synchronization height and the current timestamp
        let block_time = match assume_blocktime {
            Some(block_time) => block_time,
            None => {
                let mut bt = self.get_sync_time()?.ok_or(Error::UnsyncedWallet)?;
                bt.timestamp = crate::utils::timestamp_now();
                bt
            }
        };

        log::debug!("HeritageWallet::create_psbt - Creating foreing_utxos list");
        // We want to build 3 different informations
        // - We want the "global" Locktime to apply the transaction, essentially the maximum locktime out of all the inputs
        // - We want to keep track of all the Sequence for all the OutPoint of the inputs
        // - We want to construct the foreign UTXOs vector
        let (mut final_lock, mut seq_index, foreign_utxos) = obsolete_subwallet_configs
            .into_iter()
            .filter_map(|subwallet_config| {
                self.get_conditions_and_utxos_for_subwallet(
                    &subwallet_config,
                    &spender,
                    &block_time,
                    true,
                )
                .unwrap_or_else(|e| {
                    log::error!(
                        "HeritageWallet::create_psbt - Failed to call \
                        get_conditions_and_utxos_for_subwallet for \
                        subwallet_config {subwallet_config:?}: {e:#}"
                    );
                    None
                })
            })
            .fold(
                (
                    LockTime::from_consensus(bdk::bitcoin::absolute::LOCK_TIME_THRESHOLD),
                    HashMap::new(),
                    Vec::new(),
                ),
                |(mut final_lock, mut seq_index, mut foreign_utxos),
                 (o_locktime, o_sequence, utxos)| {
                    // Take the more restrictive locktime
                    if o_locktime
                        .is_some_and(|candidate_lock| final_lock.is_implied_by(candidate_lock))
                    {
                        final_lock = o_locktime.unwrap();
                    }
                    // Process the utxos
                    for (utxo, o_foreign_utxo) in utxos {
                        let outpoint = utxo.outpoint;
                        seq_index.insert(
                            outpoint,
                            o_sequence.unwrap_or(Sequence::ENABLE_LOCKTIME_NO_RBF),
                        );
                        let foreign_utxo =
                            o_foreign_utxo.expect("We ask to include the foreign_utxos");
                        foreign_utxos.push(foreign_utxo);
                    }
                    (final_lock, seq_index, foreign_utxos)
                },
            );

        // Create the TX builder
        log::debug!("HeritageWallet::create_psbt - wallet.build_tx()");
        let mut tx_builder = current_subwallet.build_tx();

        // We will always use offline signing so we include the redeem witness scripts
        // TODO: Verify if this is actually necessary
        log::debug!(
            "HeritageWallet::create_psbt - tx_builder.include_output_redeem_witness_script()"
        );
        tx_builder.include_output_redeem_witness_script();

        // Assume block height
        log::debug!(
            "HeritageWallet::create_psbt - tx_builder.current_height({})",
            block_time.height
        );
        tx_builder.current_height(block_time.height);

        let drain_script = match &spending_config {
            SpendingConfig::DrainTo(addr) => {
                log::debug!(
                    "HeritageWallet::create_psbt - tx_builder.drain_wallet().drain_to({addr:?})"
                );
                tx_builder.drain_wallet().drain_to(addr.script_pubkey());
                addr.script_pubkey()
            }
            SpendingConfig::Recipients(recipients) => {
                log::debug!(
                    "HeritageWallet::create_psbt - tx_builder.set_recipients({recipients:?})"
                );
                // Convert the recipients address to scripts
                let recipients = recipients
                    .iter()
                    .map(|Recipient(addr, amount)| (addr.script_pubkey(), amount.to_sat()))
                    .collect::<Vec<_>>();
                tx_builder.set_recipients(recipients);
                let drain_addr = self.internal_get_new_address(KeychainKind::Internal)?;
                tx_builder.drain_to(drain_addr.script_pubkey());
                drain_addr.script_pubkey()
            }
        };

        // Keep a set of the OutPoint corresponding to already minimized PsbtInputs to filter them out of the final minimization
        let already_minimized_psbt_input_by_outpoint = foreign_utxos
            .iter()
            .map(|(op, _, _)| *op)
            .collect::<HashSet<_>>();

        // Include all the obsolete_wallet_outputs
        log::debug!("HeritageWallet::create_psbt - tx_builder.add_foreign_utxo - foreing_utxos={foreign_utxos:?}");
        for (outpoint, psbt_input, satisfaction_weight) in foreign_utxos {
            tx_builder
                .add_foreign_utxo(outpoint, psbt_input, satisfaction_weight)
                .expect("Parameters are under our control and correct");
        }

        // Set FeeRate
        let fee_rate = self.database.borrow().get_fee_rate()?.unwrap_or_else(||{
            log::warn!("HeritageWallet::create_psbt - No FeeRate in the database. Maybe call sync_fee_rate");
            FeeRate::BROADCAST_MIN
        });
        let fee_rate = BdkFeeRate::from_sat_per_kwu(fee_rate.to_sat_per_kwu() as f32);
        log::debug!("HeritageWallet::create_psbt - tx_builder.fee_rate - fee_rate={fee_rate:?}");
        tx_builder.fee_rate(fee_rate);

        // Extract the HeritageExplorer if we are with an Heir spending and if any
        let heritage_explorer = if let Spender::Heir(heir_config) = &spender {
            current_subwallet_config
                .heritage_config()
                .get_heritage_explorer(heir_config)
        } else {
            None
        };

        // Policy for the PSBT
        let policy_index = if let Some(he) = &heritage_explorer {
            he.get_miniscript_index() + 1
        } else {
            0
        };

        log::debug!("HeritageWallet::create_psbt - policy_index={policy_index}");
        let keychains = [KeychainKind::External, KeychainKind::Internal];
        for kc in keychains {
            if let Some(pol) = current_subwallet.policies(kc).map_err(|e| match e {
                bdk::Error::InvalidPolicyPathError(e) => Error::FailToExtractPolicy(e),
                _ => {
                    log::error!(
                        "Unknown error while extracting policies from the current SubwalletConfig: {e:#}"
                    );
                    panic!(
                        "Unknown error while extracting policies from the current SubwalletConfig: {e:#}"
                    )
                }
            })? {
                let bt = BTreeMap::from([(pol.id, vec![policy_index])]);
                log::debug!(
                    "HeritageWallet::create_psbt - tx_builder.policy_path - bt={bt:?}, kc={kc:?}"
                );
                tx_builder.policy_path(bt, kc);
            }
        }

        // If the owner is spending, we let the bdk::Wallet do its work
        // but if an Heir is spending, we manually select the UTXOs we want in.
        if heir_spending {
            // Manual selection only
            log::debug!("HeritageWallet::create_psbt - tx_builder.manually_selected_only()");
            tx_builder.manually_selected_only();

            if let Some((o_locktime, o_sequence, utxos)) = self
                .get_conditions_and_utxos_for_subwallet(
                    &current_subwallet_config,
                    &spender,
                    &block_time,
                    false,
                )?
            {
                // Take the more restrictive locktime
                if o_locktime.is_some_and(|candidate_lock| final_lock.is_implied_by(candidate_lock))
                {
                    final_lock = o_locktime.unwrap();
                }
                // Process the utxos
                for (utxo, _) in utxos {
                    let outpoint = utxo.outpoint;
                    seq_index.insert(
                        outpoint,
                        o_sequence.unwrap_or(Sequence::ENABLE_LOCKTIME_NO_RBF),
                    );
                    tx_builder.add_utxo(outpoint).map_err(|e| match e {
                        bdk::Error::UnknownUtxo => {
                            log::error!("Unexpected UnknownUtxo error: {e:#}");
                            panic!("Unexpected UnknownUtxo error: {e:#}")
                        }
                        _ => Error::DatabaseError(DatabaseError::Generic(e.to_string())),
                    })?;
                }
            }
        }

        // Create the PSBT
        log::debug!("HeritageWallet::create_psbt - tx_builder.finish()");
        let (mut psbt, _) = tx_builder.finish().map_err(|e| match e {
            bdk::Error::InvalidPolicyPathError(e) => Error::FailToExtractPolicy(e),
            bdk::Error::UnknownUtxo
            | bdk::Error::FeeRateTooLow { .. }
            | bdk::Error::FeeTooLow { .. }
            | bdk::Error::ScriptDoesntHaveAddressForm
            | bdk::Error::InsufficientFunds { .. }
            | bdk::Error::NoRecipients
            | bdk::Error::NoUtxosSelected
            | bdk::Error::OutputBelowDustLimit(_)
            | bdk::Error::BnBTotalTriesExceeded
            | bdk::Error::BnBNoExactMatch
            | bdk::Error::FeeRateUnavailable
            | bdk::Error::SpendingPolicyRequired(_)
            | bdk::Error::TransactionNotFound
            | bdk::Error::Psbt(_)
            | bdk::Error::Miniscript(_)
            | bdk::Error::MiniscriptPsbt(_)
            | bdk::Error::InvalidOutpoint(_)
            | bdk::Error::InvalidNetwork { .. }
            | bdk::Error::Descriptor(_)
            | bdk::Error::ChecksumMismatch => Error::PsbtCreationError(e.to_string()),
            _ => {
                log::error!("Unknown error while creating PSBT: {e:#}");
                Error::Unknown(e.to_string())
            }
        })?;

        // Post-process the PSBT
        // We want to:
        // - Minimize the new Inputs, if any
        // - Override the Locktime, if this is an Heir spending
        // - Override all the individual nSequence, if this is an Heir spending
        for (psbt_input, tx_input) in psbt
            .inputs
            .iter_mut()
            .zip(psbt.unsigned_tx.input.iter_mut())
        {
            // Sequence of the TX Input
            if heir_spending {
                if let Some(seq) = seq_index.get(&tx_input.previous_output) {
                    log::debug!("HeritageWallet::create_psbt - For tx_input={tx_input:?} override tx_input.sequence={seq:?}");
                    tx_input.sequence = *seq;
                }
            }

            // Minimization of the PsbtInput, if necessary
            if !already_minimized_psbt_input_by_outpoint.contains(&tx_input.previous_output) {
                minimize_psbt_input_for_spender(psbt_input, heritage_explorer.as_ref());
            }
        }
        // Override the Locktime and ensure TX v2
        if heir_spending {
            log::debug!(
                "HeritageWallet::create_psbt - Override psbt.unsigned_tx.lock_time={final_lock:?}"
            );
            psbt.unsigned_tx.lock_time = final_lock;
            psbt.unsigned_tx.version = 2;
        }

        // Adjust the fee because BDK computes it with laaaaaarge margin
        // As we are only using TapRoot inputs, we can do a lot better withtout too much difficulties
        // We just have to find the "change" output
        let adjustable_output_index = if let Some(adjustable_output_index) = psbt
            .unsigned_tx
            .output
            .iter()
            .position(|o| o.script_pubkey == drain_script)
        {
            adjustable_output_index
        } else {
            log::info!("HeritageWallet::create_psbt - This psbt does not have change, adding one");
            // We are in the remote possibility where we try to send exactly the right amount
            // so that there is no need to change
            // In that case just add the output, process the psbt, and see what happen
            let drain_output = TxOut {
                value: 0,
                script_pubkey: drain_script.clone(),
            };
            psbt.unsigned_tx.output.push(drain_output);
            psbt.outputs.push(Output::default());
            let adjustable_output_index = psbt.unsigned_tx.output.len() - 1;
            adjustable_output_index
        };

        log::debug!("HeritageWallet::create_psbt - adjust_with_real_fee(psbt, {fee_rate:?}, {adjustable_output_index})");
        let adjustment = adjust_with_real_fee(&mut psbt, &fee_rate, adjustable_output_index);
        log::info!("HeritageWallet::create_psbt - Fee adjustment: {adjustment}");

        // If the resulting amount is below dust treshold, just pop the output (and therefor give that amount to the miners)
        if psbt.unsigned_tx.output[adjustable_output_index]
            .value
            .is_dust(&drain_script)
        {
            // In that case, the adjustment is 0 because the only way we are here
            // is that we where in the case "remote possibility where we try to send exactly the right amount"
            // and we added the output drain and it happens to be too small so we just go back to the old fee.
            psbt.unsigned_tx.output.remove(adjustable_output_index);
            psbt.outputs.remove(adjustable_output_index);
        }

        // Our PSBT only contains owned inputs
        // Adding all inputs into the owned_inputs Vec
        let owned_inputs = psbt
            .inputs
            .iter()
            .map(|i| {
                let utxo = i.witness_utxo.as_ref().expect("we only deal with Taproot");
                TransactionSummaryOwnedIO(
                    (&utxo.script_pubkey)
                        .try_into()
                        .expect("comes from the PSBT"),
                    Amount::from_sat(utxo.value),
                )
            })
            .collect::<Vec<_>>();
        // Creating the owned_outputs Vec
        let owned_outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|&o| self.is_mine(o.script_pubkey.as_script()).unwrap_or(false))
            .map(|o| {
                TransactionSummaryOwnedIO(
                    (&o.script_pubkey).try_into().expect("comes from the PSBT"),
                    Amount::from_sat(o.value),
                )
            })
            .collect::<Vec<_>>();

        // Create the parent_ids
        let parent_txids = psbt
            .unsigned_tx
            .input
            .iter()
            .map(|i| i.previous_output.txid)
            .collect();

        // Create the TransactionSummary
        let tx_summary = TransactionSummary {
            txid: psbt.unsigned_tx.txid(),
            confirmation_time: None,
            owned_inputs,
            owned_outputs,
            fee: psbt.fee().expect("our psbt is fresh and sound"),
            parent_txids,
        };

        log::debug!("HeritageWallet::create_psbt - psbt={psbt:?}");
        log::debug!("HeritageWallet::create_psbt - tx_summary={tx_summary:?}");
        Ok((psbt, tx_summary))
    }

    fn get_conditions_and_utxos_for_subwallet(
        &self,
        subwallet_config: &SubwalletConfig,
        spender: &Spender,
        assume_blocktime: &BlockTime,
        include_foreign_utxo: bool,
    ) -> Result<
        Option<(
            Option<LockTime>,
            Option<Sequence>,
            Vec<(LocalUtxo, Option<(OutPoint, Input, usize)>)>,
        )>,
    > {
        log::debug!(
            "HeritageWallet::filter_subwallet_utxos - subwallet_config={subwallet_config:?} spender={spender:?} assume_blocktime={assume_blocktime:?}"
        );

        let heritage_explorer = if let Spender::Heir(heir_config) = &spender {
            let he = subwallet_config
                .heritage_config()
                .get_heritage_explorer(heir_config);
            if he.is_none() {
                // If None, the Heir is not in this wallet
                // So skip it
                return Ok(None);
            }
            he
        } else {
            None
        };

        let spend_condition = heritage_explorer
            .as_ref()
            .map(|he| he.get_spend_conditions());

        // At this point we KNOW that there is either a Some(spend_condition) or this is the owner spending
        // If it is a Some (so an Heir is spending), verify that the timestamp is compatible with they spending
        if let Some(sc) = &spend_condition {
            // If too soon, the Heir cannot yet spend this wallet
            // So skip it and return a None
            if !sc.can_spend_at(assume_blocktime.timestamp) {
                return Ok(None);
            }
        }

        let (o_lock_time, o_sequence) = match &spend_condition {
            Some(sc) => (
                sc.get_spendable_timestamp().map(|ts| {
                    LockTime::from_time(ts as u32)
                        .expect("comes from HeritageConfig which check the value")
                }),
                sc.get_relative_block_lock()
                    .map(|h| Sequence::from_height(h)),
            ),
            None => (None, None),
        };

        let subwallet = self.get_subwallet(&subwallet_config)?;
        let unspent = subwallet
            .list_unspent()
            .map_err(|e| DatabaseError::Generic(e.to_string()))?;

        // Manually select UTXOs
        let utxos_and_inputs = unspent
        .into_iter()
        // Filter to keep only UTXO spendable by the Spender
        .filter(|utxo| {
            // We want to get the confirmation time of the TX and verify that it is compatible
            // with the HeritageConfig for the Heir, i.e. that the Heir will be able to spend the UTXO
            // Their is two requirements:
            //   1. timestamp.now() must be greater than the heritagedate for the heir
            //   2. for each TX, blockheight < current_block_height + min_lock
            if let Spender::Heir(_) = spender {
                // There is a spend_condition_tester, use it to filter TX
                let tx = match subwallet.get_tx(&utxo.outpoint.txid, false) {
                    Ok(Some(tx)) => tx,
                    Ok(None) => {
                        log::error!("HeritageWallet::create_psbt_for_subwallet - Database inconsistent for Subwallet ID={} No tx with in database for {utxo:?}", subwallet_config.subwallet_id());
                        return false;
                    }
                    Err(e) => {
                        log::error!("HeritageWallet::create_psbt_for_subwallet - Failed to retrieve tx for Subwallet ID={} and {utxo:?}: {e:#}", subwallet_config.subwallet_id());
                        return false;
                    }
                };
                let Some(tx_confirmation_time) = tx.confirmation_time else {
                    return false;
                };

                // If the TX is not old enough, skip it
                if tx_confirmation_time.height
                    + spend_condition
                        .as_ref()
                        .expect("this is some when Spender::Heir")
                        .get_relative_block_lock().unwrap_or(0)
                        as u32
                    > assume_blocktime.height
                {
                    return false;
                }
            }
            true
        })
        .map(|utxo|{
            if include_foreign_utxo {
                let outpoint = utxo.outpoint;
                let mut input = subwallet.get_psbt_input(utxo.clone(), None, true).map_err(|e| match e {
                    bdk::Error::UnknownUtxo => {
                        log::error!("Unexpected UnknownUtxo error: {e:#}");
                        panic!("Unexpected UnknownUtxo error: {e:#}")
                    }
                    bdk::Error::MiniscriptPsbt(_) => Error::PsbtCreationError(e.to_string()),
                    _ => DatabaseError::Generic(e.to_string()).into(),
                })?;
                minimize_psbt_input_for_spender(&mut input, heritage_explorer.as_ref());
                let satisfaction_weight = subwallet
                .get_descriptor_for_keychain(utxo.keychain)
                .max_weight_to_satisfy()
                .expect("our descriptors can always be satisfied");
                Ok((utxo, Some((outpoint, input, satisfaction_weight))))
            } else {
                Ok((utxo, None))
            }
        }).filter_map(|r: Result<_>|{
            match r {
                Ok(v) => Some(v),
                Err(e) => {
                    log::error!("HeritageWallet::create_psbt_for_subwallet - Failed to create PsbtInput for Subwallet ID={}: {e:#}", subwallet_config.subwallet_id());
                    None
                }
            }
        })
        .collect::<Vec<_>>();

        // If there are no applicable UTXO, don't bother sending a result
        let result = if utxos_and_inputs.len() == 0 {
            None
        } else {
            Some((o_lock_time, o_sequence, utxos_and_inputs))
        };
        log::debug!("HeritageWallet::filter_subwallet_utxos - result={result:?}");
        Ok(result)
    }

    fn create_new_subwallet_config(
        &self,
        heritage_config: HeritageConfig,
        old_subwallet_config: Option<SubwalletConfig>,
    ) -> Result<()> {
        log::debug!(
            "HeritageWallet::create_new_subwallet_config - old_subwallet_config={old_subwallet_config:?}"
        );
        // If different, then we need to archive the old subwallet_config and create a new one
        // With a new AccountXPub
        let new_account_xpub = self
            .database
            .borrow()
            .get_unused_account_xpub()?
            .ok_or(Error::MissingUnusedAccountXPub)?;
        log::debug!(
            "HeritageWallet::update_heritage_config - new_account_xpub={new_account_xpub:?}"
        );
        let mut transaction = self.database.borrow().begin_transac();
        transaction.delete_unused_account_xpub(&new_account_xpub)?;
        let new_subwallet_config = SubwalletConfig::new(new_account_xpub, heritage_config);
        log::info!("HeritageWallet::update_heritage_config - Creating a new SubwalletConfig for the new HeritageConfig");
        log::debug!(
            "HeritageWallet::update_heritage_config - new_subwallet_config={new_subwallet_config:?}"
        );
        transaction.safe_update_current_subwallet_config(
            &new_subwallet_config,
            old_subwallet_config.as_ref(),
        )?;
        if let Some(old_subwallet_config) = old_subwallet_config {
            transaction.put_subwallet_config(
                SubwalletConfigId::Id(old_subwallet_config.subwallet_id()),
                &old_subwallet_config,
            )?;
        }
        self.database.borrow_mut().commit_transac(transaction)?;
        Ok(())
    }

    fn get_subwallet(
        &self,
        subwalletconfig: &SubwalletConfig,
    ) -> Result<Wallet<<D as PartitionableDatabase>::SubDatabase>> {
        log::debug!("HeritageWallet::get_subwallet - Opening subwallet database");
        let subdatabase = self
            .database
            .borrow()
            .get_subdatabase(SubdatabaseId::from(subwalletconfig.subwallet_id()))?;
        log::debug!("HeritageWallet::get_subwallet - Creating subwallet");
        Ok(subwalletconfig.get_subwallet(subdatabase))
    }

    fn internal_get_new_address(&self, keychain_kind: KeychainKind) -> Result<AddressInfo> {
        log::debug!("HeritageWallet::internal_get_new_address - keychain_kind={keychain_kind:?}");

        let current_subwallet_config = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
            .ok_or(Error::MissingCurrentSubwalletConfig)?;
        log::debug!("HeritageWallet::internal_get_new_address - current_subwallet_config={current_subwallet_config:?}");

        // Verify if the subwallet_config has not already been used, just override it
        if current_subwallet_config.subwallet_firstuse_time().is_none() {
            log::debug!(
                "HeritageWallet::internal_get_new_address - current_subwallet_config was never used"
            );
            let mut new_current_subwallet_config = current_subwallet_config.clone();
            new_current_subwallet_config.mark_subwallet_firstuse()?;
            log::info!("HeritageWallet::internal_get_new_address - Marking previously unused SubwalletConfig as used");
            log::debug!(
                "HeritageWallet::internal_get_new_address - new_current_subwallet_config={new_current_subwallet_config:?}"
            );
            self.database
                .borrow_mut()
                .safe_update_current_subwallet_config(
                    &new_current_subwallet_config,
                    Some(&current_subwallet_config),
                )?;
        }
        log::debug!("HeritageWallet::internal_get_new_address - get_subwallet");
        let subwallet = self.get_subwallet(&current_subwallet_config)?;

        let address = match keychain_kind {
            KeychainKind::External => {
                log::debug!("HeritageWallet::internal_get_new_address - subwallet.get_address");
                subwallet
                    .get_address(AddressIndex::New)
                    .map_err(|e| match e {
                        bdk::Error::ScriptDoesntHaveAddressForm => Error::Unknown(
                            "ScriptDoesntHaveAddressForm: Invalid script retrieved from database"
                                .to_owned(),
                        ),
                        _ => DatabaseError::Generic(e.to_string()).into(),
                    })?
            }
            KeychainKind::Internal => {
                log::debug!(
                    "HeritageWallet::internal_get_new_address - subwallet.get_internal_address"
                );
                subwallet
                    .get_internal_address(AddressIndex::New)
                    .map_err(|e| match e {
                        bdk::Error::ScriptDoesntHaveAddressForm => Error::Unknown(
                            "ScriptDoesntHaveAddressForm: Invalid script retrieved from database"
                                .to_owned(),
                        ),
                        _ => DatabaseError::Generic(e.to_string()).into(),
                    })?
            }
        };
        log::debug!("HeritageWallet::internal_get_new_address - address={address:?}");
        Ok(address)
    }
}

/// Take a mutable reference to a [Input] and make sure that it does not contains superfluous
/// informations that are not needed by the intended spender.
/// The idea is to remove from the [Input] all the scripts and key paths that are
/// not relevant for the spender.
/// It also enhence privacy, because Heirs will not be able to know who are
/// the other Heirs from the Psbt. At most, an Heir can infere how many Heirs were
/// before him by looking at the number of merkle_branches necessary for them to
/// create their transaction because of the way we create the MAST
/// (see comments in [HeritageConfig]::descriptor_segment())
///
/// This function SHOULD become largelly useless once [bdk] starts using miniscript v11.0
/// because it introduces a clean way to generate a minimized Psbt from the get-go, as well
/// as a feature to better compute the weight of the future transaction (currently, [bdk] uses
/// the worst-case weight, meaning eveybody pays the fee as if they were the last Heir, not ideal)
fn minimize_psbt_input_for_spender(
    psbt_input: &mut Input,
    heritage_explorer: Option<&HeritageExplorer>,
) {
    log::debug!("minimize_psbt_for_spender - heritage_explorer={heritage_explorer:?}");
    match heritage_explorer {
        // This is the owner spending
        None => {
            // With the owner it is simple: simply clean the scripts
            psbt_input.tap_scripts.clear();
            // Then remove every Key that is not the tap_internal_key
            psbt_input.tap_key_origins.retain(|k, _| {
                *k == psbt_input
                    .tap_internal_key
                    .expect("this is a taproot input")
            })
        }
        // This is an Heir spending
        Some(heritage_explorer) => {
            // Keeps only the relevant keys
            psbt_input
                .tap_key_origins
                .retain(|_, (_, (fingerprint, _))| heritage_explorer.has_fingerprint(*fingerprint));
            // Create a Vec of the retained origins
            let origins = psbt_input
                .tap_key_origins
                .iter()
                .map(|(_, (_, (fingerprint, derivation_path)))| (fingerprint, derivation_path));
            // Use the origins to ask for a concrete Script
            let script = heritage_explorer.get_script(origins);
            // Keeps only the relevant script
            psbt_input.tap_scripts.retain(|_, (s, _)| *s == script);
        }
    }
}

/// Take a mutable reference to a [Psbt] and compute the exact expected weight of the final transaction.
/// It is possible to do so relatively easily since:
/// 1. the [Psbt] is for a Taproot SegWit TX
/// 2. We already minimized each [Input] down to a single spend-path so that we can easily see if it is
/// a key-path or script-path and in the later case, we know what will be the size of the control block
/// by infering the depth of the script in the MAST.
///
/// After that, we use the given [FeeRate] to compute the Fee and adjust the amount of the
/// [bdk::bitcoin::psbt::Output] pointed by `adjustable_output_index`.
///
/// This function MAY become largelly useless once [bdk] starts using miniscript v11.0
/// because it introduces a clean way to generate a minimized Psbt from the get-go, as well
/// as a feature to better compute the weight of the future transaction (currently, [bdk] uses
/// the worst-case weight, meaning eveybody pays the fee as if they were the last Heir, not ideal)
///
/// But on the other hand, bdk is really doing kind-of a shitty job computing the WU of the future TX:
/// 1. It uses a wrong [bdk::wallet::coin_selection::TXIN_BASE_WEIGHT] because it forget that the absence of signature
/// must still be signaled by setting sig_length to 0, which consumme 1 byte / 4 WU. The weight of the TX is thus
/// underestimated by 4WU/input
/// 2. On the other hand, it uses the obsolete [Tr::max_satisfaction_weight] which compensate that
/// 3. On the other other hand, it always assume a witness_stack size of the worst-case scenario, meaning it uses the size
/// the witness stack would have by spending the deepest/longest script in the MAST. In case of a Key-path spend
/// it ugely overestimates the TX weight, in case of script-path spend, it depends on the script we actually intend to
/// use but tend to overestimate also.
///
/// All in all, that makes the TX weight often wrong and thus the Fee too.
///
/// # Returns
/// Return an i64 telling the amount of adjustment made to the adjustable_output
///
/// # Panics
/// Panics if the [Psbt] contains [Input] that are not Taproot, or if the Psbt Inputs where
/// not minimized (see [minimize_psbt_input_for_spender])
fn adjust_with_real_fee(
    psbt: &mut Psbt,
    fee_rate: &BdkFeeRate,
    adjustable_output_index: usize,
) -> i64 {
    log::debug!(
        "adjust_with_real_fee - psbt={psbt:?} fee_rate={fee_rate:?} \
        adjustable_output_index={adjustable_output_index}"
    );

    let expected_weight = get_expected_tx_weight(&psbt);
    log::debug!("adjust_with_real_fee - expected_weight={expected_weight:#}");
    log::debug!(
        "adjust_with_real_fee - expected_weight={} vB (ceil)",
        expected_weight.to_vbytes_ceil()
    );

    // Compute the new_fee using the expected_weight and the fee_rate
    let new_fee = fee_rate.fee_wu(expected_weight);
    let current_fee = psbt
        .fee()
        .expect("the PSBT is assumed to be valid")
        .to_sat();

    log::debug!("adjust_with_real_fee - current_fee={current_fee} sat; new_fee={new_fee} sat");
    // Compute the change we need to the adjustable_output to get the correct fee
    // In case we have to lower the amount, we could endup having a amount <0
    // or a Dust amount. In either case, warn and do nothing
    if new_fee > current_fee {
        // We need to lower the amount in adjustable_output to increase the fee
        let adjustment = new_fee - current_fee;
        if let Some(new_amount) = psbt.unsigned_tx.output[adjustable_output_index]
            .value
            .checked_sub(adjustment)
        {
            if new_amount.is_dust(&psbt.unsigned_tx.output[adjustable_output_index].script_pubkey)
                && new_amount != 0
            {
                log::warn!(
                    "adjust_with_real_fee - current_fee={current_fee} is lower than the \
                new_fee={new_fee} but the resulting new_amount {new_amount} for the \
                adjustable_output would be dust. Do nothing."
                );
                0
            } else {
                psbt.unsigned_tx.output[adjustable_output_index].value = new_amount;
                -(adjustment as i64)
            }
        } else {
            log::warn!(
                "adjust_with_real_fee - current_fee={current_fee} is lower than the \
            new_fee={new_fee} but the resulting new_amount for the adjustable_output \
            would be negative. Do nothing."
            );
            0
        }
    } else {
        let adjustment = current_fee - new_fee;
        psbt.unsigned_tx.output[adjustable_output_index].value += adjustment;
        adjustment as i64
    }
}

fn get_expected_tx_weight(psbt: &Psbt) -> Weight {
    log::debug!("get_expected_tx_weight - psbt={psbt}");
    // Put some barriers so we do not misuses this
    // Ensure this is a pure Taproot Psbt
    assert!(psbt
        .inputs
        .iter()
        .all(|input| input.tap_internal_key.is_some() && input.tap_merkle_root.is_some()));

    // The [Weight] is the addition of:
    // - The TX weight (without any script_sig or witness at this point)
    // - The 2 additionnal WU coming from the segwit format (marker + flag)
    // - Expected witness and sig size for each input

    // TX weight given by rust-bitcoin implementation, nice
    let mut expected_weight = psbt.unsigned_tx.weight();
    // Fixed addition of the 2 additionnal WU coming from the segwit format
    expected_weight += Weight::from_wu(2);
    // Expected witness and sig size for each input
    expected_weight += psbt
        .inputs
        .iter()
        .map(|input| {
            match input.tap_scripts.len() {
                // Code here is copied from <rust-miniscript 10.0.0>/src/descriptor/tr.rs
                // {} impl Tr<Pk>::max_weight_to_satisfy(&self)
                0 => {
                    // key spend path
                    // item: varint(sig+sigHash) + <sig(64)+sigHash(1)>
                    let item_sig_size = 1 + 65;
                    // 1 stack item
                    // let stack_varint_diff = varint_len(1) - varint_len(0); // Always 0

                    item_sig_size
                }
                1 => {
                    // Script spend
                    let (ctr_block, (script, _)) = input.tap_scripts.first_key_value().unwrap();
                    let miniscript: Miniscript<_, Tap> = Miniscript::parse(script).unwrap();
                    let script_size = miniscript.script_size();
                    let max_sat_elems = miniscript
                        .max_satisfaction_witness_elements()
                        .expect("Our Miniscript are satisfyable");
                    let max_sat_size = miniscript
                        .max_satisfaction_size()
                        .expect("Our Miniscript are satisfyable");
                    let control_block_size = ctr_block.size();

                    // stack varint difference (+1 for ctrl block, witness script already included)
                    let stack_varint_diff = varint_len(max_sat_elems + 1) - varint_len(0);

                    stack_varint_diff +
                        // size of elements to satisfy script
                        max_sat_size +
                        // second to last element: script
                        varint_len(script_size) +
                        script_size +
                        // last element: control block
                        varint_len(control_block_size) +
                        control_block_size
                }
                _ => panic!("Psbt input is not minimized"),
            }
        })
        .fold(Weight::ZERO, |acc, iw| {
            acc + Weight::from_witness_data_size(iw as u64)
        });
    expected_weight
}

// Helper function to calculate witness size
// copied from <rust-miniscript 10.0.0>/src/utils.rs
fn varint_len(n: usize) -> usize {
    bdk::bitcoin::VarInt(n as u64).len()
}

#[cfg(test)]
mod tests {

    use core::{cell::RefCell, str::FromStr};
    use std::collections::{hash_map::RandomState, HashMap, HashSet};

    use bdk::{
        blockchain::{
            Blockchain, BlockchainFactory, Capability, GetBlockHash, GetHeight, GetTx, Progress,
            WalletSync,
        },
        database::{BatchDatabase, SyncTime},
        Balance, BlockTime, Error, FeeRate, KeychainKind, LocalUtxo, TransactionDetails,
    };

    use crate::{
        bitcoin::{
            absolute::LockTime,
            bip32::{DerivationPath, Fingerprint},
            secp256k1::XOnlyPublicKey,
            taproot::TapNodeHash,
            Amount, BlockHash, OutPoint, Sequence, Transaction, Txid,
        },
        database::{memory::HeritageMemoryDatabase, HeritageDatabase, TransacHeritageOperation},
        heritage_wallet::{
            backup::{HeritageWalletBackup, SubwalletDescriptorBackup},
            get_expected_tx_weight, BlockInclusionObjective, HeritageWallet, HeritageWalletBalance,
            Recipient, SpendingConfig, SubwalletConfigId,
        },
        miniscript::{Descriptor, DescriptorPublicKey},
        tests::*,
        utils::{extract_tx, string_to_address},
        HeritageConfig,
    };

    #[derive(Debug, Clone)]
    pub struct FakeBlockchain {
        pub current_height: BlockTime,
        pub transactions: Vec<TransactionDetails>,
    }

    impl GetHeight for FakeBlockchain {
        fn get_height(&self) -> Result<u32, Error> {
            Ok(self.current_height.height)
        }
    }
    impl GetTx for FakeBlockchain {
        fn get_tx(&self, _txid: &Txid) -> Result<Option<Transaction>, Error> {
            Err(Error::Generic("Unimplemented".to_owned()))
        }
    }
    impl GetBlockHash for FakeBlockchain {
        fn get_block_hash(&self, _height: u64) -> Result<BlockHash, Error> {
            Err(Error::Generic("Unimplemented".to_owned()))
        }
    }
    impl WalletSync for FakeBlockchain {
        fn wallet_setup<D: BatchDatabase>(
            &self,
            database: &RefCell<D>,
            _progress_update: Box<dyn Progress>,
        ) -> Result<(), Error> {
            for tx_details in &self.transactions {
                database.borrow_mut().set_tx(tx_details)?;
                database.borrow_mut().set_utxo(&LocalUtxo {
                    txout: tx_details.transaction.as_ref().unwrap().output[0].clone(),
                    outpoint: OutPoint {
                        txid: tx_details.txid.clone(),
                        vout: 0,
                    },
                    keychain: KeychainKind::External,
                    is_spent: false,
                })?;
            }
            database.borrow_mut().set_sync_time(SyncTime {
                block_time: self.current_height.clone(),
            })?;
            Ok(())
        }
    }
    impl Blockchain for FakeBlockchain {
        fn get_capabilities(&self) -> HashSet<Capability> {
            [
                Capability::FullHistory,
                Capability::AccurateFees,
                Capability::GetAnyTx,
            ]
            .into()
        }

        fn broadcast(&self, _tx: &Transaction) -> Result<(), Error> {
            Err(Error::Generic("Unimplemented".to_owned()))
        }

        fn estimate_fee(&self, _target: usize) -> Result<FeeRate, Error> {
            Ok(FeeRate::from_sat_per_vb(10.0))
        }
    }

    #[derive(Debug, Clone)]
    pub struct FakeBlockchainFactory {
        pub current_height: BlockTime,
    }
    impl BlockchainFactory for FakeBlockchainFactory {
        type Inner = FakeBlockchain;

        fn build(
            &self,
            wallet_name: &str,
            _override_skip_blocks: Option<u32>,
        ) -> Result<Self::Inner, bdk::Error> {
            let mut hashtable: HashMap<String, Vec<TransactionDetails>> = HashMap::new();
            // Wallet TestHeritageConfig::BackupWifeY2
            hashtable.insert("7y7nqca9j84snf2h".to_owned(), vec![
                serde_json::from_str(r#"{"transaction":{"version":1,"lock_time":0,"input":[{"previous_output":"0000000000000000000000000000000000000000000000000000000000000000:4294967295","script_sig":"","sequence":4294967295,"witness":[]}],"output":[{"value":100000000,"script_pubkey":"51208bdbdb2969eeb7ec8efd20f7bc64961a760313404e900109a1ba19afb8b0292c"}]},"txid":"344dbc396e3c6945f46a67faab275141bb0fdd63f8a46362ba27e4753400d9c2","received":100000000,"sent":0,"fee":0,"confirmation_time":{"height":842520,"timestamp":1715552000}}"#).unwrap(),
                serde_json::from_str(r#"{"transaction":{"version":1,"lock_time":0,"input":[{"previous_output":"0000000000000000000000000000000000000000000000000000000000000000:4294967295","script_sig":"","sequence":4294967295,"witness":[]}],"output":[{"value":100000000,"script_pubkey":"51204eff92d75cc954964dfe21755cf3063abbe19aa0e6fcacf429af996fc0f65beb"}]},"txid":"d2f3bd44fb6ad0c32833ea943d718e806245e632302f25720811fea167c13507","received":100000000,"sent":0,"fee":0,"confirmation_time":{"height":904440,"timestamp":1752704000}}"#).unwrap(),
            ]);
            // Wallet TestHeritageConfig::BackupWifeY1
            hashtable.insert("0hqx0prur5t9us5w".to_owned(), vec![
                serde_json::from_str(r#"{"transaction":{"version":1,"lock_time":0,"input":[{"previous_output":"0000000000000000000000000000000000000000000000000000000000000000:4294967295","script_sig":"","sequence":4294967295,"witness":[]}],"output":[{"value":100000000,"script_pubkey":"5120a6d2ae7fb6a453f32d1d32ffdfc31f3303a7704bcf16ff4ebabe0e26686ec687"}]},"txid":"2f0a77d510db56dda3b43692d4658a92f523193a3b854d2387681f2fd0f5d920","received":100000000,"sent":0,"fee":0,"confirmation_time":{"height":895080,"timestamp":1747088000}}"#).unwrap(),
                serde_json::from_str(r#"{"transaction":{"version":1,"lock_time":0,"input":[{"previous_output":"0000000000000000000000000000000000000000000000000000000000000000:4294967295","script_sig":"","sequence":4294967295,"witness":[]}],"output":[{"value":100000000,"script_pubkey":"5120f0c155ea564bd2fc6d45f654a68d2734b762a947d93c4cf6e2fdda0260ee89d9"}]},"txid":"3854db1cb2253a270e49a093a6ddb92fa79efd8b295568e08448e4de678fc08b","received":100000000,"sent":0,"fee":0,"confirmation_time":{"height":897960,"timestamp":1748816000}}"#).unwrap(),
            ]);
            // Wallet TestHeritageConfig::BackupWifeBro
            hashtable.insert("9lwn0wm9mh7ydv64".to_owned(), vec![
                serde_json::from_str(r#"{"transaction":{"version":1,"lock_time":0,"input":[{"previous_output":"0000000000000000000000000000000000000000000000000000000000000000:4294967295","script_sig":"","sequence":4294967295,"witness":[]}],"output":[{"value":100000000,"script_pubkey":"5120d1756b2e88a51fc63f156ef0ba3a3cfd126206bfb96347f1b4ccb15f9b75e14f"}]},"txid":"6ed1563a936196211f2f76447c478533df8f3efc43933f4c3405b9a760b31204","received":100000000,"sent":0,"fee":0,"confirmation_time":{"height":923160,"timestamp":1763936000}}"#).unwrap(),
            ]);

            Ok(FakeBlockchain {
                current_height: self.current_height.clone(),
                transactions: hashtable
                    .get(wallet_name)
                    .map(Clone::clone)
                    .unwrap_or_default(),
            })
        }
    }

    fn setup_wallet() -> HeritageWallet<HeritageMemoryDatabase> {
        let mut db = HeritageMemoryDatabase::new();

        // Account descriptors
        let unused_axps = (3..10)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect();
        db.add_unused_account_xpubs(&unused_axps).unwrap();

        // Wallet subconfigs
        db.put_subwallet_config(
            SubwalletConfigId::Id(0),
            &get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY2),
        )
        .unwrap();
        db.put_subwallet_config(
            SubwalletConfigId::Id(1),
            &get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY1),
        )
        .unwrap();
        db.put_subwallet_config(
            SubwalletConfigId::Current,
            &get_default_test_subwallet_config(TestHeritageConfig::BackupWifeBro),
        )
        .unwrap();

        let wallet = HeritageWallet::new(db);
        wallet
            .sync(FakeBlockchainFactory {
                current_height: get_present(),
            })
            .unwrap();

        wallet
    }

    fn get_present() -> BlockTime {
        get_blocktime_for_timestamp(
            get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeY1,
                TestHeritage::Backup,
            ) + 86400 * 15, // 15 days after backup can inherit, therefore wife cannot yet.
        )
    }

    #[test]
    fn get_balance() {
        let wallet = setup_wallet();

        let expected_balance = HeritageWalletBalance::new(
            Balance {
                confirmed: 100_000_000,
                ..Default::default()
            },
            Balance {
                confirmed: 400_000_000,
                ..Default::default()
            },
        );
        assert_eq!(wallet.get_balance().unwrap(), expected_balance);
    }

    #[test]
    fn fingerprint() {
        // Test on an empty wallet
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        // An empty wallet does not have a fingerprint
        assert!(wallet.fingerprint().is_ok_and(|f| f.is_none()));

        // Test on an non-empty wallet
        let wallet = setup_wallet();
        // A non-empty wallet does have a fingerprint
        assert_eq!(
            wallet.fingerprint().unwrap(),
            Some(
                get_test_account_xpub(0)
                    .descriptor_public_key()
                    .master_fingerprint()
            )
        );
    }

    #[test]
    fn list_used_account_xpubs() {
        let wallet = setup_wallet();
        let expected = (0..3)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect::<Vec<_>>();
        assert_eq!(wallet.list_used_account_xpubs().unwrap(), expected)
    }

    #[test]
    fn generate_backup() {
        let wallet = setup_wallet();
        // To have a last_external_index on the last backup
        let _ = wallet.get_new_address().unwrap();
        // We expect the values set in the tests mod of lib.rs
        let expected = HeritageWalletBackup(vec![
            SubwalletDescriptorBackup {
                external_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_external_descriptor(
                        TestHeritageConfig::BackupWifeY2,
                    ),
                )
                .unwrap(),
                change_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_change_descriptor(
                        TestHeritageConfig::BackupWifeY2,
                    ),
                )
                .unwrap(),
                first_use_ts: get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY2)
                    .subwallet_firstuse_time(),
                last_external_index: None,
                last_change_index: None,
            },
            SubwalletDescriptorBackup {
                external_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_external_descriptor(
                        TestHeritageConfig::BackupWifeY1,
                    ),
                )
                .unwrap(),
                change_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_change_descriptor(
                        TestHeritageConfig::BackupWifeY1,
                    ),
                )
                .unwrap(),
                first_use_ts: get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY1)
                    .subwallet_firstuse_time(),
                last_external_index: None,
                last_change_index: None,
            },
            SubwalletDescriptorBackup {
                external_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_external_descriptor(
                        TestHeritageConfig::BackupWifeBro,
                    ),
                )
                .unwrap(),
                change_descriptor: Descriptor::<DescriptorPublicKey>::from_str(
                    get_default_test_subwallet_config_expected_change_descriptor(
                        TestHeritageConfig::BackupWifeBro,
                    ),
                )
                .unwrap(),
                first_use_ts: get_default_test_subwallet_config(TestHeritageConfig::BackupWifeBro)
                    .subwallet_firstuse_time(),
                last_external_index: Some(0),
                last_change_index: None,
            },
        ]);
        assert_eq!(wallet.generate_backup().unwrap(), expected)
    }

    #[test]
    fn restore_backup() {
        let wallet = setup_wallet();
        // To have a last_external_index on the last backup
        let _ = wallet.get_new_address().unwrap();

        // We expect that if we backup and then restore (i.e. duplicates) the wallet,
        // we will have effectively the same wallet (same balance, same addresses, etc...)

        let new_wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        let backup = wallet.generate_backup().unwrap();

        // Restoration goes ok
        let r = new_wallet.restore_backup(backup);
        assert!(r.is_ok(), "{}", r.err().unwrap());

        // New address from both wallet are the same
        assert_eq!(
            wallet.get_new_address().unwrap(),
            new_wallet.get_new_address().unwrap()
        );

        // Sync
        new_wallet
            .sync(FakeBlockchainFactory {
                current_height: get_present(),
            })
            .unwrap();

        // Balance from both wallet are the same
        assert_eq!(
            wallet.get_balance().unwrap(),
            new_wallet.get_balance().unwrap()
        );

        // Trying to restore another time should fail
        assert!(new_wallet
            .restore_backup(wallet.generate_backup().unwrap())
            .is_err());
    }

    #[test]
    fn list_wallet_addresses() {
        // Empty wallet
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        // Add AccountXPubs
        wallet
            .append_account_xpubs((0..3).into_iter().map(|i| get_test_account_xpub(i)))
            .unwrap();
        // Test the expected sequence of addresses
        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .unwrap();
        wallet.get_new_address().unwrap();
        wallet.get_new_address().unwrap();

        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY1))
            .unwrap();
        wallet.get_new_address().unwrap();
        wallet.get_new_address().unwrap();

        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeBro))
            .unwrap();
        wallet.get_new_address().unwrap();
        wallet.get_new_address().unwrap();

        let results = wallet.list_wallet_addresses().unwrap();

        // Expected addresses, in order
        let expected_addresses = vec![
            get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeBro,
                1,
            ),
            get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeBro,
                0,
            ),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY1, 1),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY1, 0),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 1),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 0),
        ];

        assert_eq!(
            results
                .iter()
                .map(|wa| wa.address().to_string())
                .collect::<Vec<_>>(),
            expected_addresses
        );

        // The fingerprint should be the same for every WalletAddress origin
        let fingerprint = get_test_account_xpub(0)
            .descriptor_public_key()
            .master_fingerprint();
        assert!(results.iter().all(|wa| wa.origin().0 == fingerprint));

        // Expected derivation paths, in order
        let expected_derivation_paths = (0..3)
            .map(|a_i| {
                let axpub = get_test_account_xpub(a_i);
                let dp = axpub
                    .descriptor_public_key()
                    .full_derivation_path()
                    .unwrap();
                dp.normal_children()
                    .take(1)
                    .map(|cdp| cdp.normal_children().take(2).collect::<Vec<_>>())
                    .flatten()
                    .collect::<Vec<_>>()
            })
            .flatten()
            .rev()
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .map(|wa| wa.origin().1.clone())
                .collect::<Vec<_>>(),
            expected_derivation_paths
        );
    }

    #[test]
    fn list_unused_account_xpubs() {
        let wallet = setup_wallet();
        let expected = (3..10)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect::<Vec<_>>();
        assert_eq!(wallet.list_unused_account_xpubs().unwrap(), expected)
    }

    #[test]
    fn append_account_xpubs() {
        let wallet = setup_wallet();
        let initial_used = wallet.list_used_account_xpubs().unwrap();
        let mut initial_unused = wallet.list_unused_account_xpubs().unwrap();
        let to_add = (0..15).into_iter().map(|i| get_test_account_xpub(i));
        let expected_add = (10..15)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect::<Vec<_>>();

        // Append the descriptors
        assert!(wallet.append_account_xpubs(to_add).is_ok());

        // Used ADs do not change
        assert_eq!(wallet.list_used_account_xpubs().unwrap(), initial_used);

        // Unused ADs change and is now initial_unused + expected_add
        initial_unused.extend(expected_add);
        assert_eq!(wallet.list_unused_account_xpubs().unwrap(), initial_unused);

        // Cannot add an Account XPub with a different fingerprint
        assert!(wallet
            .append_account_xpubs([get_bad_account_xpub()])
            .is_err_and(|e| match e {
                crate::errors::Error::InvalidAccountXPub => true,
                _ => false,
            }));
    }

    #[test]
    fn get_current_heritage_config() {
        let wallet = setup_wallet();
        assert_eq!(
            wallet.get_current_heritage_config().unwrap().unwrap(),
            get_test_heritage_config(TestHeritageConfig::BackupWifeBro)
        );
    }

    #[test]
    fn list_obsolete_heritage_configs() {
        let wallet = setup_wallet();
        let expected = vec![
            get_test_heritage_config(TestHeritageConfig::BackupWifeY2),
            get_test_heritage_config(TestHeritageConfig::BackupWifeY1),
        ];
        assert_eq!(wallet.list_obsolete_heritage_configs().unwrap(), expected);
    }

    #[test]
    fn update_heritage_config() {
        // Test on an empty wallet
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        wallet
            .append_account_xpubs((0..5).into_iter().map(|i| get_test_account_xpub(i)))
            .unwrap();
        // No current wallet
        assert!(wallet.get_current_heritage_config().unwrap().is_none());
        // Update is ok
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .is_ok());
        // Current wallet is now the new one
        assert_eq!(
            wallet.get_current_heritage_config().unwrap().unwrap(),
            get_test_heritage_config(TestHeritageConfig::BackupWifeY2)
        );
        // An account descriptor has been consumed
        let used_axps = wallet.list_used_account_xpubs().unwrap();
        assert!(used_axps.len() == 1 && used_axps[0] == get_test_account_xpub(0));
        let unused_axps = wallet.list_unused_account_xpubs().unwrap();
        assert!(unused_axps.len() == 4 && unused_axps[0] == get_test_account_xpub(1));
        // Now generate an address
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY2,
                0
            )));

        // As it as been used, replacing it should cause the creation of an obsolete HeritageConfig
        // Update is ok
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY1))
            .is_ok());
        // Current wallet is now the new one
        assert_eq!(
            wallet.get_current_heritage_config().unwrap().unwrap(),
            get_test_heritage_config(TestHeritageConfig::BackupWifeY1)
        );
        // An account descriptor has been consumed
        let used_axps = wallet.list_used_account_xpubs().unwrap();
        assert!(used_axps.len() == 2 && used_axps[1] == get_test_account_xpub(1));
        let unused_axps = wallet.list_unused_account_xpubs().unwrap();
        assert!(unused_axps.len() == 3 && unused_axps[0] == get_test_account_xpub(2));
        // Obsolete HeritageConfig
        let obsolete_heritage_config = wallet.list_obsolete_heritage_configs().unwrap();
        assert_eq!(obsolete_heritage_config.len(), 1);
        assert_eq!(
            obsolete_heritage_config[0],
            get_test_heritage_config(TestHeritageConfig::BackupWifeY2)
        );

        // As it as NOT been used, replacing it should NOT cause the creation of an obsolete HeritageConfig
        // Update is ok
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeBro))
            .is_ok());
        // Current wallet is now the new one
        assert_eq!(
            wallet.get_current_heritage_config().unwrap().unwrap(),
            get_test_heritage_config(TestHeritageConfig::BackupWifeBro)
        );
        // An account descriptor has NOT been consumed, i.e. used_axps.len == 2
        let used_axps = wallet.list_used_account_xpubs().unwrap();
        assert!(used_axps.len() == 2);
        let unused_axps = wallet.list_unused_account_xpubs().unwrap();
        assert!(unused_axps.len() == 3 && unused_axps[0] == get_test_account_xpub(2));
        // Obsolete HeritageConfig is still len==1 and contains only BackupWifeY2
        let obsolete_heritage_config = wallet.list_obsolete_heritage_configs().unwrap();
        assert_eq!(obsolete_heritage_config.len(), 1);
        assert_eq!(
            obsolete_heritage_config[0],
            get_test_heritage_config(TestHeritageConfig::BackupWifeY2)
        );

        // Now generate an address
        assert!(wallet.get_new_address().is_ok());
        // Replace with the same HeritageConfig
        // Update is ok
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeBro))
            .is_ok());
        // Current wallet is now the new one
        assert_eq!(
            wallet.get_current_heritage_config().unwrap().unwrap(),
            get_test_heritage_config(TestHeritageConfig::BackupWifeBro)
        );
        // An account descriptor has NOT been consumed, i.e. used_axps.len == 2
        let used_axps = wallet.list_used_account_xpubs().unwrap();
        assert!(used_axps.len() == 2);
        let unused_axps = wallet.list_unused_account_xpubs().unwrap();
        assert!(unused_axps.len() == 3 && unused_axps[0] == get_test_account_xpub(2));
        // Obsolete HeritageConfig is still len==1 and contains only BackupWifeY2
        let obsolete_heritage_config = wallet.list_obsolete_heritage_configs().unwrap();
        assert_eq!(obsolete_heritage_config.len(), 1);
        assert_eq!(
            obsolete_heritage_config[0],
            get_test_heritage_config(TestHeritageConfig::BackupWifeY2)
        );

        // Test on wallet with only one AD
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        wallet
            .append_account_xpubs((0..1).into_iter().map(|i| get_test_account_xpub(i)))
            .unwrap();
        // Update is ok
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .is_ok());
        // Now generate an address
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY2,
                0
            )));
        // Update fail with new HeritageConfig because no more ADs
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY1))
            .is_err());
        // Update succeed with the same HeritageConfig
        assert!(wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .is_ok());
    }

    #[test]
    fn get_new_address() {
        // Test on an empty wallet
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        // Add AccountXPubs
        wallet
            .append_account_xpubs((0..3).into_iter().map(|i| get_test_account_xpub(i)))
            .unwrap();
        // Test the expected sequence of addresses
        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY2,
                0
            )));
        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY2))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY2,
                1,
            )));

        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY1))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY1,
                0
            )));
        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeY1))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeY1,
                1,
            )));

        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeBro))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeBro,
                0
            )));
        wallet
            .update_heritage_config(get_test_heritage_config(TestHeritageConfig::BackupWifeBro))
            .unwrap();
        assert!(wallet.get_new_address().is_ok_and(|addr| addr.to_string()
            == get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeBro,
                1,
            )));
    }

    #[test]
    fn get_set_block_inclusion_objective() {
        // Test on an empty wallet
        let wallet = HeritageWallet::new(HeritageMemoryDatabase::new());
        assert_eq!(
            wallet.get_block_inclusion_objective().unwrap(),
            BlockInclusionObjective::default()
        );
        let new_bio = BlockInclusionObjective::from(50u16);
        assert!(wallet.set_block_inclusion_objective(new_bio).is_ok());
        assert_eq!(wallet.get_block_inclusion_objective().unwrap(), new_bio);

        let new_bio = BlockInclusionObjective::from(30u16);
        assert!(wallet.set_block_inclusion_objective(new_bio).is_ok());
        assert_eq!(wallet.get_block_inclusion_objective().unwrap(), new_bio);
    }

    #[test]
    fn wallet_first_use_time() {
        let wallet = setup_wallet();
        // Create a brand new HeritageConfig (which does not really make sense but we don't care for this test)
        wallet
            .update_heritage_config(
                HeritageConfig::builder()
                    .add_heritage(get_test_heritage(TestHeritage::Backup).time_lock(180))
                    .reference_time(get_present().timestamp)
                    .minimum_lock_time(10)
                    .build(),
            )
            .unwrap();

        // At this stage, the current wallet is unused
        assert!(wallet
            .database()
            .get_subwallet_config(SubwalletConfigId::Current)
            .unwrap()
            .unwrap()
            .subwallet_firstuse_time()
            .is_none());

        wallet.get_new_address().unwrap();

        // At this stage, the current wallet is used
        assert!(wallet
            .database()
            .get_subwallet_config(SubwalletConfigId::Current)
            .unwrap()
            .unwrap()
            .subwallet_firstuse_time()
            .is_some());

        // Create a brand new HeritageConfig (which does not really make sense but we don't care for this test)
        wallet
            .update_heritage_config(
                HeritageConfig::builder()
                    .add_heritage(get_test_heritage(TestHeritage::Backup).time_lock(360))
                    .reference_time(get_present().timestamp)
                    .minimum_lock_time(30)
                    .build(),
            )
            .unwrap();
        // // Sync
        // wallet
        //     .sync(FakeBlockchainFactory {
        //         current_height: get_present(),
        //     })
        //     .unwrap();

        // At this stage, the current wallet is unused
        assert!(wallet
            .database()
            .get_subwallet_config(SubwalletConfigId::Current)
            .unwrap()
            .unwrap()
            .subwallet_firstuse_time()
            .is_none());

        wallet
            .create_owner_psbt(SpendingConfig::Recipients(vec![Recipient(
                string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                Amount::from_sat(10_000),
            )]))
            .unwrap();

        // At this stage, the current wallet is used
        assert!(wallet
            .database()
            .get_subwallet_config(SubwalletConfigId::Current)
            .unwrap()
            .unwrap()
            .subwallet_firstuse_time()
            .is_some());
    }

    #[test]
    fn list_heritage_utxos() {
        let wallet = setup_wallet();
        let res = wallet.database().list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        let hus = res.unwrap();
        assert_eq!(hus.len(), 5);

        // Control 2 UTXOs
        // For this UTXO, the expected timestamps are the absolute timelocks expiration for each Heir
        assert!(hus.iter().any(|hu| {
            hu.confirmation_time.as_ref().unwrap().height == 897960
                && hu.amount.to_sat() == 100_000_000
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Backup).get_heir_config(),
                ) == Some(1763072000)
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Wife).get_heir_config(),
                ) == Some(1766096000)
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Brother).get_heir_config(),
                ) == None
        }));

        // For this UTXO, the expected timestamps are estimates based on the 90/180 days relative
        // lock after the transaction is confirmed (transaction came in long after the absoltue locktime expiration)
        assert!(hus.iter().any(|hu| {
            hu.confirmation_time.as_ref().unwrap().height == 904440
                && hu.amount.to_sat() == 100_000_000
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Backup).get_heir_config(),
                ) == Some(1760480000)
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Wife).get_heir_config(),
                ) == Some(1768256000)
                && hu.estimate_heir_spending_timestamp(
                    get_test_heritage(TestHeritage::Brother).get_heir_config(),
                ) == None
        }));
    }

    #[test]
    fn list_transaction_summaries() {
        let wallet = setup_wallet();
        let res = wallet.database().list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        let tx_sums = res.unwrap();
        assert_eq!(tx_sums.len(), 5);
        for tx_sum in tx_sums.iter() {
            println!("{tx_sum:?}");
        }
        assert!(tx_sums.iter().all(|txs| txs.owned_outputs.len() == 1
            && txs.owned_outputs[0].1 == Amount::from_btc(1.0).unwrap()
            && txs.owned_inputs.len() == 0));
    }

    #[test]
    fn create_owner_psbt_recipient() {
        let wallet = setup_wallet();
        let (psbt, tx_sum) = wallet
            .create_owner_psbt(SpendingConfig::Recipients(vec![
                Recipient::from((
                    string_to_address(PKH_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(0.1).unwrap(),
                )),
                Recipient::from((
                    string_to_address(WPKH_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(0.2).unwrap(),
                )),
                Recipient::from((
                    string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(0.3).unwrap(),
                )),
            ]))
            .unwrap();

        // This PSBT has 4 inputs, corresponding to the 4 obsolete transaction
        // The 5th tx (in the current_subwallet) is not used because the drain
        // of the obsolete subwallets completely covers the outputs
        assert_eq!(psbt.inputs.len(), 4);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 2], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "a9a963f530557632c19635f7719bef70d31c09a1294b3d13e20d38c65913e19c",
                    )
                    .unwrap(),
                    [
                        "05a793ed9e1755bd27f404b27cd1fce931ba8451a6fd060678268ca4a24b15e5",
                        "m/86'/1'/0'/0/1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "m/86'/1'/0'/0/0",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "56c4e0d07b07f3bbe2e2e7ccc581e3e23104f5f504dd026942707a491fd031aa",
                    )
                    .unwrap(),
                    [
                        "2a5046a9fdcb12b3cfab38da4266c6fad729d96153f80de2fa5af27726b5269e",
                        "m/86'/1'/1'/0/0",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "619b3cf5c3b8b19807262b9a17fa29784be05b029ccb4b4e8723cf3a23dfbb33",
                    )
                    .unwrap(),
                    [
                        "693880a19260132e9360318040fa32c3f95ae6180394d8b09e09fa67c215ee8c",
                        "m/86'/1'/1'/0/1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is for the tap_internal_key (owner is spending)
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(&input.tap_internal_key.unwrap())
            .is_some_and(
                |(tap_leaf_hash, (key_fingerprint, derivation_path))| tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("9c7088e3").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[1]
                        )
                        .unwrap()
            )
            && input.tap_key_origins.len() == 1));
        // There is no scripts (owner is spending)
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.is_empty()));
        // 4 outputs
        assert_eq!(psbt.outputs.len(), 4);
        // Output with 10_000_000 sat is P2PKH
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value == 10_000_000)
            .all(|e| e.script_pubkey.is_p2pkh()));
        // Output with 20_000_000 sat is P2WPKH
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value == 20_000_000)
            .all(|e| e.script_pubkey.is_v0_p2wpkh()));
        // Output with 30_000_000 sat and more are P2TR
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value >= 30_000_000)
            .all(|e| e.script_pubkey.is_v1_p2tr()));

        // The output with ~ 340_000_000 sat is the change and is owned by the current subwallet
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value >= 300_000_000)
            .all(|e| wallet.is_mine_and_current(&e.script_pubkey).unwrap()));

        // PSBT is exactly the expected one if we sort the outputs
        let mut psbt = psbt;
        let mut tmp = psbt
            .unsigned_tx
            .output
            .into_iter()
            .zip(psbt.outputs.into_iter())
            .collect::<Vec<_>>();
        tmp.sort_by(|a, b| a.0.script_pubkey.cmp(&b.0.script_pubkey));
        (psbt.unsigned_tx.output, psbt.outputs) = tmp.into_iter().unzip();

        let mut expected_psbt = get_test_unsigned_psbt(TestPsbt::OwnerRecipients);
        let mut tmp = expected_psbt
            .unsigned_tx
            .output
            .into_iter()
            .zip(expected_psbt.outputs.into_iter())
            .collect::<Vec<_>>();
        tmp.sort_by(|a, b| a.0.script_pubkey.cmp(&b.0.script_pubkey));
        (expected_psbt.unsigned_tx.output, expected_psbt.outputs) = tmp.into_iter().unzip();

        assert_eq!(psbt, expected_psbt);

        assert_eq!(tx_sum.confirmation_time, None);

        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(3.39996080).unwrap()
        );
        // Uses all "old" UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(4.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00003920).unwrap());
    }

    #[test]
    fn create_owner_psbt_drains_to() {
        let wallet = setup_wallet();
        let (psbt, tx_sum) = wallet
            .create_owner_psbt(SpendingConfig::DrainTo(
                string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
            ))
            .unwrap();

        // This PSBT has 5 inputs
        assert_eq!(psbt.inputs.len(), 5);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 2], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "a9a963f530557632c19635f7719bef70d31c09a1294b3d13e20d38c65913e19c",
                    )
                    .unwrap(),
                    [
                        "05a793ed9e1755bd27f404b27cd1fce931ba8451a6fd060678268ca4a24b15e5",
                        "m/86'/1'/0'/0/1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "m/86'/1'/0'/0/0",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "56c4e0d07b07f3bbe2e2e7ccc581e3e23104f5f504dd026942707a491fd031aa",
                    )
                    .unwrap(),
                    [
                        "2a5046a9fdcb12b3cfab38da4266c6fad729d96153f80de2fa5af27726b5269e",
                        "m/86'/1'/1'/0/0",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "619b3cf5c3b8b19807262b9a17fa29784be05b029ccb4b4e8723cf3a23dfbb33",
                    )
                    .unwrap(),
                    [
                        "693880a19260132e9360318040fa32c3f95ae6180394d8b09e09fa67c215ee8c",
                        "m/86'/1'/1'/0/1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "1ebb6d6824ff497e82e65952788975af7dfa6e921109157f379000667d2f4a76",
                    )
                    .unwrap(),
                    [
                        "017779241278e5d108dd9ad60ba75fbfbe68fc1accc6a54be15a8ebff9de0f0c",
                        "m/86'/1'/2'/0/0",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is for the tap_internal_key (owner is spending)
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(&input.tap_internal_key.unwrap())
            .is_some_and(
                |(tap_leaf_hash, (key_fingerprint, derivation_path))| tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("9c7088e3").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[1]
                        )
                        .unwrap()
            )
            && input.tap_key_origins.len() == 1));
        // There is no scripts (owner is spending)
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.is_empty()));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::OwnerDrain));

        assert_eq!(tx_sum.confirmation_time, None);

        // Receive nothing, draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses all "old" UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(5.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00003410).unwrap());
    }

    #[test]
    fn create_backup_heir_psbt() {
        let wallet = setup_wallet();
        let heir_config = get_test_heritage(TestHeritage::Backup)
            .get_heir_config()
            .clone();
        // Heirs MUST use the Drain SpendingConfig
        assert!(wallet
            .create_heir_psbt(
                heir_config.clone(),
                SpendingConfig::Recipients(vec![Recipient::from((
                    string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(1.0).unwrap(),
                ))]),
                Some(get_present())
            )
            .is_err());
        let (psbt, tx_sum) = wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(get_present()),
            )
            .unwrap();

        // This PSBT has 4 inputs
        assert_eq!(psbt.inputs.len(), 4);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 4], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20",
                        "m/86'/1'/1751476594'/0/0",
                        "205dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20ad02a032b2690480243567b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "a9a963f530557632c19635f7719bef70d31c09a1294b3d13e20d38c65913e19c",
                    )
                    .unwrap(),
                    [
                        "05a793ed9e1755bd27f404b27cd1fce931ba8451a6fd060678268ca4a24b15e5",
                        "807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130",
                        "m/86'/1'/1751476594'/0/1",
                        "20807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130ad02a032b2690480243567b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "56c4e0d07b07f3bbe2e2e7ccc581e3e23104f5f504dd026942707a491fd031aa",
                    )
                    .unwrap(),
                    [
                        "2a5046a9fdcb12b3cfab38da4266c6fad729d96153f80de2fa5af27726b5269e",
                        "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20",
                        "m/86'/1'/1751476594'/0/0",
                        "205dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20ad02a032b2690400581669b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "619b3cf5c3b8b19807262b9a17fa29784be05b029ccb4b4e8723cf3a23dfbb33",
                    )
                    .unwrap(),
                    [
                        "693880a19260132e9360318040fa32c3f95ae6180394d8b09e09fa67c215ee8c",
                        "807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130",
                        "m/86'/1'/1751476594'/0/1",
                        "20807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130ad02a032b2690400581669b1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is the heir key
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[1],
                )
                .unwrap(),
            )
            .is_some_and(|(tap_leaf_hash, (key_fingerprint, derivation_path))| {
                !tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("f0d79bf6").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[2],
                        )
                        .unwrap()
            })
            && input.tap_key_origins.len() == 1));
        // There is one script and its the one expected for the heir
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.len() == 1
            && input.tap_scripts.first_key_value().is_some_and(|(_, v)| {
                v.0.to_hex_string()
                    == expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[3]
            })));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // TX is v2
        assert_eq!(psbt.unsigned_tx.version, 2);
        // TX has the expected Locktime
        assert_eq!(
            psbt.unsigned_tx.lock_time,
            LockTime::from_time(get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeY1,
                TestHeritage::Backup,
            ) as u32)
            .unwrap()
        );
        // TX inputs have the expected sequence
        assert!(psbt
            .unsigned_tx
            .input
            .iter()
            .all(|input| input.sequence.enables_absolute_lock_time() && input.sequence.0 == 12960));

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::BackupPresent));

        assert_eq!(tx_sum.confirmation_time, None);
        // Receive nothing, heir is draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses all "old" UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(4.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00003960).unwrap());
    }

    #[test]
    fn create_wife_heir_psbt() {
        let wallet = setup_wallet();
        let heir_config = get_test_heritage(TestHeritage::Wife)
            .get_heir_config()
            .clone();
        // Heirs MUST use the Drain SpendingConfig
        assert!(wallet
            .create_heir_psbt(
                heir_config.clone(),
                SpendingConfig::Recipients(vec![Recipient::from((
                    string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(1.0).unwrap(),
                ))]),
                Some(get_present())
            )
            .is_err());
        let (psbt, tx_sum) = wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(get_present()),
            )
            .unwrap();

        // This PSBT has 1 inputs
        assert_eq!(psbt.inputs.len(), 1);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 4], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b2690400496367b1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is the heir key
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[1],
                )
                .unwrap(),
            )
            .is_some_and(|(tap_leaf_hash, (key_fingerprint, derivation_path))| {
                !tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("c907dcb9").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[2],
                        )
                        .unwrap()
            })
            && input.tap_key_origins.len() == 1));
        // There is one script and its the one expected for the heir
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.len() == 1
            && input.tap_scripts.first_key_value().is_some_and(|(_, v)| {
                v.0.to_hex_string()
                    == expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[3]
            })));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // TX is v2
        assert_eq!(psbt.unsigned_tx.version, 2);
        // TX has the expected Locktime
        assert_eq!(
            psbt.unsigned_tx.lock_time,
            LockTime::from_time(get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeY2,
                TestHeritage::Wife,
            ) as u32)
            .unwrap()
        );
        // TX inputs have the expected sequence
        assert!(psbt
            .unsigned_tx
            .input
            .iter()
            .all(|input| input.sequence.enables_absolute_lock_time() && input.sequence.0 == 25920));

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::WifePresent));

        assert_eq!(tx_sum.confirmation_time, None);
        // Receive nothing, heir is draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses only the eligible UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(1.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00001390).unwrap());
    }

    #[test]
    fn create_brother_heir_psbt() {
        let wallet = setup_wallet();
        let heir_config = get_test_heritage(TestHeritage::Brother)
            .get_heir_config()
            .clone();
        // Heirs MUST use the Drain SpendingConfig
        assert!(wallet
            .create_heir_psbt(
                heir_config.clone(),
                SpendingConfig::Recipients(vec![Recipient::from((
                    string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap(),
                    Amount::from_btc(1.0).unwrap(),
                ))]),
                Some(get_present())
            )
            .is_err());
        // Brother cannot spend anything yet so that should fail PSBT generation
        assert!(wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(get_present()),
            )
            .is_err());
    }

    /// Far future is interesting to test for 2 reasons
    /// 1. It allow verifying the PSBT when there is more than 2 heirs, and therefore the MAST is depth>1
    /// 2. It allow to verify how the PSBT generation behave when spending from the "current" wallet,
    /// which should only happen if the owner is dead...
    #[test]
    fn create_backup_heir_psbt_far_future() {
        let far_future =
            get_blocktime_for_timestamp(get_present().timestamp + 10 * 365 * 24 * 60 * 60);
        let wallet = setup_wallet();
        wallet
            .sync(FakeBlockchainFactory {
                current_height: far_future.clone(),
            })
            .unwrap();
        let heir_config = get_test_heritage(TestHeritage::Backup)
            .get_heir_config()
            .clone();
        let (psbt, tx_sum) = wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(far_future),
            )
            .unwrap();

        // This PSBT has 5 inputs
        assert_eq!(psbt.inputs.len(), 5);

        let expected_values: HashMap<XOnlyPublicKey, [&str; 4], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20",
                        "m/86'/1'/1751476594'/0/0",
                        "205dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20ad02a032b2690480243567b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "a9a963f530557632c19635f7719bef70d31c09a1294b3d13e20d38c65913e19c",
                    )
                    .unwrap(),
                    [
                        "05a793ed9e1755bd27f404b27cd1fce931ba8451a6fd060678268ca4a24b15e5",
                        "807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130",
                        "m/86'/1'/1751476594'/0/1",
                        "20807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130ad02a032b2690480243567b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "56c4e0d07b07f3bbe2e2e7ccc581e3e23104f5f504dd026942707a491fd031aa",
                    )
                    .unwrap(),
                    [
                        "2a5046a9fdcb12b3cfab38da4266c6fad729d96153f80de2fa5af27726b5269e",
                        "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20",
                        "m/86'/1'/1751476594'/0/0",
                        "205dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20ad02a032b2690400581669b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "619b3cf5c3b8b19807262b9a17fa29784be05b029ccb4b4e8723cf3a23dfbb33",
                    )
                    .unwrap(),
                    [
                        "693880a19260132e9360318040fa32c3f95ae6180394d8b09e09fa67c215ee8c",
                        "807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130",
                        "m/86'/1'/1751476594'/0/1",
                        "20807065fea1488df239f9bc5b295f4a4f03eb8b3ddbe5e96335efb54476f71130ad02a032b2690400581669b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "1ebb6d6824ff497e82e65952788975af7dfa6e921109157f379000667d2f4a76",
                    )
                    .unwrap(),
                    [
                        "017779241278e5d108dd9ad60ba75fbfbe68fc1accc6a54be15a8ebff9de0f0c",
                        "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20",
                        "m/86'/1'/1751476594'/0/0",
                        "205dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20ad02a032b26904808bf76ab1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is the heir key
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[1],
                )
                .unwrap(),
            )
            .is_some_and(|(tap_leaf_hash, (key_fingerprint, derivation_path))| {
                !tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("f0d79bf6").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[2],
                        )
                        .unwrap()
            })
            && input.tap_key_origins.len() == 1));
        // There is one script and its the one expected for the heir
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.len() == 1
            && input.tap_scripts.first_key_value().is_some_and(|(_, v)| {
                v.0.to_hex_string()
                    == expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[3]
            })));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // TX is v2
        assert_eq!(psbt.unsigned_tx.version, 2);
        // TX has the expected Locktime
        assert_eq!(
            psbt.unsigned_tx.lock_time,
            LockTime::from_time(get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeBro,
                TestHeritage::Backup,
            ) as u32)
            .unwrap()
        );
        // TX inputs have the expected sequence
        assert!(psbt
            .unsigned_tx
            .input
            .iter()
            .all(|input| input.sequence.enables_absolute_lock_time() && input.sequence.0 == 12960));

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::BackupFuture));

        assert_eq!(tx_sum.confirmation_time, None);
        // Receive nothing, heir is draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses only the eligible UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(5.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00004810).unwrap());
    }

    #[test]
    fn create_wife_heir_psbt_far_future() {
        let far_future =
            get_blocktime_for_timestamp(get_present().timestamp + 10 * 365 * 24 * 60 * 60);
        let wallet = setup_wallet();
        wallet
            .sync(FakeBlockchainFactory {
                current_height: far_future.clone(),
            })
            .unwrap();
        let heir_config = get_test_heritage(TestHeritage::Wife)
            .get_heir_config()
            .clone();
        let (psbt, tx_sum) = wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(far_future),
            )
            .unwrap();

        // This PSBT has 5 input
        assert_eq!(psbt.inputs.len(), 5);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 4], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177",
                    )
                    .unwrap(),
                    [
                        "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b2690400496367b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "a9a963f530557632c19635f7719bef70d31c09a1294b3d13e20d38c65913e19c",
                    )
                    .unwrap(),
                    [
                        "05a793ed9e1755bd27f404b27cd1fce931ba8451a6fd060678268ca4a24b15e5",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b2690400496367b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "56c4e0d07b07f3bbe2e2e7ccc581e3e23104f5f504dd026942707a491fd031aa",
                    )
                    .unwrap(),
                    [
                        "2a5046a9fdcb12b3cfab38da4266c6fad729d96153f80de2fa5af27726b5269e",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b26904807c4469b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "619b3cf5c3b8b19807262b9a17fa29784be05b029ccb4b4e8723cf3a23dfbb33",
                    )
                    .unwrap(),
                    [
                        "693880a19260132e9360318040fa32c3f95ae6180394d8b09e09fa67c215ee8c",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b26904807c4469b1",
                    ],
                ),
                (
                    XOnlyPublicKey::from_str(
                        "1ebb6d6824ff497e82e65952788975af7dfa6e921109157f379000667d2f4a76",
                    )
                    .unwrap(),
                    [
                        "017779241278e5d108dd9ad60ba75fbfbe68fc1accc6a54be15a8ebff9de0f0c",
                        "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf",
                        "m/86'/1'/1751476594'/0/0",
                        "209d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bfad024065b2690400b0256bb1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is the heir key
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[1],
                )
                .unwrap(),
            )
            .is_some_and(|(tap_leaf_hash, (key_fingerprint, derivation_path))| {
                !tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("c907dcb9").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[2],
                        )
                        .unwrap()
            })
            && input.tap_key_origins.len() == 1));
        // There is one script and its the one expected for the heir
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.len() == 1
            && input.tap_scripts.first_key_value().is_some_and(|(_, v)| {
                v.0.to_hex_string()
                    == expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[3]
            })));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // TX is v2
        assert_eq!(psbt.unsigned_tx.version, 2);
        // TX has the expected Locktime
        assert_eq!(
            psbt.unsigned_tx.lock_time,
            LockTime::from_time(get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeBro,
                TestHeritage::Wife,
            ) as u32)
            .unwrap()
        );
        // TX inputs have the expected sequence
        assert!(psbt
            .unsigned_tx
            .input
            .iter()
            .all(|input| input.sequence.enables_absolute_lock_time() && input.sequence.0 == 25920));

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::WifeFuture));

        assert_eq!(tx_sum.confirmation_time, None);
        // Receive nothing, heir is draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses only the eligible UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(5.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00004890).unwrap());
    }

    #[test]
    fn create_brother_heir_psbt_far_future() {
        let far_future =
            get_blocktime_for_timestamp(get_present().timestamp + 10 * 365 * 24 * 60 * 60);
        let wallet = setup_wallet();
        wallet
            .sync(FakeBlockchainFactory {
                current_height: far_future.clone(),
            })
            .unwrap();
        let heir_config = get_test_heritage(TestHeritage::Brother)
            .get_heir_config()
            .clone();
        let (psbt, tx_sum) = wallet
            .create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(string_to_address(TR_EXTERNAL_RECIPIENT_ADDR).unwrap()),
                Some(far_future),
            )
            .unwrap();

        // This PSBT has 1 input
        assert_eq!(psbt.inputs.len(), 1);
        let expected_values: HashMap<XOnlyPublicKey, [&str; 4], RandomState> =
            HashMap::from_iter(vec![
                (
                    XOnlyPublicKey::from_str(
                        "1ebb6d6824ff497e82e65952788975af7dfa6e921109157f379000667d2f4a76",
                    )
                    .unwrap(),
                    [
                        "017779241278e5d108dd9ad60ba75fbfbe68fc1accc6a54be15a8ebff9de0f0c",
                        "f49679ef0089dda208faa970d7491cca8334bbe2ca541f527a6d7adf06a53e9e",
                        "m/86'/1'/1751476594'/0/0",
                        "20f49679ef0089dda208faa970d7491cca8334bbe2ca541f527a6d7adf06a53e9ead03e09700b2690480d4536bb1",
                    ],
                ),
            ]);
        // Tap Internal keys are expected
        assert!(psbt.inputs.iter().all(|input| input
            .tap_internal_key
            .is_some_and(|ik| expected_values.contains_key(&ik))));
        // Tap Merkle roots are expected
        assert!(psbt
            .inputs
            .iter()
            .all(|input| input.tap_merkle_root.is_some_and(|tnh| tnh
                == TapNodeHash::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[0],
                )
                .unwrap())));
        // There is one key path per input and it is the heir key
        assert!(psbt.inputs.iter().all(|input| input
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[1],
                )
                .unwrap(),
            )
            .is_some_and(|(tap_leaf_hash, (key_fingerprint, derivation_path))| {
                !tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("767e581a").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str(
                            expected_values
                                .get(&input.tap_internal_key.unwrap())
                                .unwrap()[2],
                        )
                        .unwrap()
            })
            && input.tap_key_origins.len() == 1));
        // There is one script and its the one expected for the heir
        assert!(psbt.inputs.iter().all(|input| input.tap_scripts.len() == 1
            && input.tap_scripts.first_key_value().is_some_and(|(_, v)| {
                v.0.to_hex_string()
                    == expected_values
                        .get(&input.tap_internal_key.unwrap())
                        .unwrap()[3]
            })));
        // 1 output
        assert_eq!(psbt.outputs.len(), 1);

        // TX is v2
        assert_eq!(psbt.unsigned_tx.version, 2);
        // TX has the expected Locktime
        assert_eq!(
            psbt.unsigned_tx.lock_time,
            LockTime::from_time(get_absolute_inheritance_timestamp(
                TestHeritageConfig::BackupWifeBro,
                TestHeritage::Brother,
            ) as u32)
            .unwrap()
        );
        // TX inputs have the expected sequence
        assert!(psbt
            .unsigned_tx
            .input
            .iter()
            .all(|input| input.sequence.enables_absolute_lock_time() && input.sequence.0 == 38880));

        // PSBT is exactly the expected one
        assert_eq!(psbt, get_test_unsigned_psbt(TestPsbt::BrotherFuture));

        assert_eq!(tx_sum.confirmation_time, None);
        // Receive nothing, heir is draining
        assert_eq!(
            tx_sum.owned_outputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(0.0).unwrap()
        );
        // Uses only the eligible UTXO
        assert_eq!(
            tx_sum.owned_inputs.iter().map(|o| o.1).sum::<Amount>(),
            Amount::from_btc(1.0).unwrap()
        );
        assert_eq!(tx_sum.fee, Amount::from_btc(0.00001480).unwrap());
    }

    fn _tx_weight_prediction(tp: TestPsbt) {
        let unsigned_psbt = get_test_unsigned_psbt(tp);
        let signed_psbt = get_test_signed_psbt(tp);
        let tx = extract_tx(signed_psbt).unwrap();
        assert_eq!(tx.weight(), get_expected_tx_weight(&unsigned_psbt));
    }
    #[test]
    fn tx_weight_prediction() {
        _tx_weight_prediction(TestPsbt::OwnerRecipients);
        _tx_weight_prediction(TestPsbt::OwnerDrain);
        _tx_weight_prediction(TestPsbt::BackupFuture);
        _tx_weight_prediction(TestPsbt::BackupPresent);
        _tx_weight_prediction(TestPsbt::WifeFuture);
        _tx_weight_prediction(TestPsbt::WifePresent);
        _tx_weight_prediction(TestPsbt::BrotherFuture);
    }

    fn _owner_signed_psbt_finalization(tp: TestPsbt) {
        let tx = extract_tx(get_test_signed_psbt(tp)).unwrap();
        // TX is v1
        assert_eq!(tx.version, 1);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_height());
        assert_eq!(
            tx.lock_time,
            LockTime::from_height(get_present().height).unwrap()
        );
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == Sequence::ENABLE_LOCKTIME_NO_RBF));
    }

    #[test]
    fn owner_signed_psbt_finalization() {
        _owner_signed_psbt_finalization(TestPsbt::OwnerRecipients);
        _owner_signed_psbt_finalization(TestPsbt::OwnerDrain);
    }

    #[test]
    fn heir_signed_backup_present_psbt_finalization() {
        let tx = extract_tx(get_test_signed_psbt(TestPsbt::BackupPresent)).unwrap();
        let expected_lock_time = LockTime::from_time(get_absolute_inheritance_timestamp(
            TestHeritageConfig::BackupWifeY1,
            TestHeritage::Backup,
        ) as u32)
        .unwrap();
        let expected_sequence = Sequence::from_height(12960);
        // TX is v2
        assert_eq!(tx.version, 2);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_time());
        assert_eq!(tx.lock_time, expected_lock_time);
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == expected_sequence));
    }

    #[test]
    fn heir_signed_wife_present_psbt_finalization() {
        let tx = extract_tx(get_test_signed_psbt(TestPsbt::WifePresent)).unwrap();
        let expected_lock_time = LockTime::from_time(get_absolute_inheritance_timestamp(
            TestHeritageConfig::BackupWifeY2,
            TestHeritage::Wife,
        ) as u32)
        .unwrap();
        let expected_sequence = Sequence::from_height(25920);
        // TX is v2
        assert_eq!(tx.version, 2);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_time());
        assert_eq!(tx.lock_time, expected_lock_time);
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == expected_sequence));
    }

    #[test]
    fn heir_signed_backup_future_psbt_finalization() {
        let tx = extract_tx(get_test_signed_psbt(TestPsbt::BackupFuture)).unwrap();
        let expected_lock_time = LockTime::from_time(get_absolute_inheritance_timestamp(
            TestHeritageConfig::BackupWifeBro,
            TestHeritage::Backup,
        ) as u32)
        .unwrap();
        let expected_sequence = Sequence::from_height(12960);
        // TX is v2
        assert_eq!(tx.version, 2);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_time());
        assert_eq!(tx.lock_time, expected_lock_time);
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == expected_sequence));
    }

    #[test]
    fn heir_signed_wife_future_psbt_finalization() {
        let tx = extract_tx(get_test_signed_psbt(TestPsbt::WifeFuture)).unwrap();
        let expected_lock_time = LockTime::from_time(get_absolute_inheritance_timestamp(
            TestHeritageConfig::BackupWifeBro,
            TestHeritage::Wife,
        ) as u32)
        .unwrap();
        let expected_sequence = Sequence::from_height(25920);
        // TX is v2
        assert_eq!(tx.version, 2);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_time());
        assert_eq!(tx.lock_time, expected_lock_time);
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == expected_sequence));
    }

    #[test]
    fn heir_signed_brother_future_psbt_finalization() {
        let tx = extract_tx(get_test_signed_psbt(TestPsbt::BrotherFuture)).unwrap();
        let expected_lock_time = LockTime::from_time(get_absolute_inheritance_timestamp(
            TestHeritageConfig::BackupWifeBro,
            TestHeritage::Brother,
        ) as u32)
        .unwrap();
        let expected_sequence = Sequence::from_height(38880);
        // TX is v2
        assert_eq!(tx.version, 2);
        // TX has the expected Locktime
        assert!(tx.lock_time.is_block_time());
        assert_eq!(tx.lock_time, expected_lock_time);
        assert!(tx
            .input
            .iter()
            .all(|input| input.sequence == expected_sequence));
    }
}
