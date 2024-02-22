use std::collections::{HashMap, HashSet};

use bdk::{
    bitcoin::{Amount, OutPoint, Txid},
    blockchain::{log_progress, Blockchain, BlockchainFactory},
    Balance, FeeRate, SyncOptions,
};

use super::{
    HeritageUtxo, HeritageWallet, HeritageWalletBalance, SubwalletConfigId, TransactionSummary,
};
use crate::{
    database::TransacHeritageDatabase,
    errors::{DatabaseError, Error, Result},
    subwalletconfig::SubwalletConfig,
};

impl<D: TransacHeritageDatabase> HeritageWallet<D> {
    pub fn sync<T: BlockchainFactory>(&self, blockchain_factory: T) -> Result<()> {
        log::debug!("HeritageWallet::sync");
        // Manage the HeritageUtxo updates
        let mut existing_utxos = self.database().list_utxos()?;
        let mut utxos_to_add = vec![];
        let mut utxos_to_delete = vec![];
        // Manage the TransactionSummary updates
        let mut txsum_to_add = HashMap::new();
        // Start obsolete_balance at zero
        let mut obsolete_balance = Balance::default();
        // Walk over every subwallets and sync them
        let subwalletconfigs = self.database.borrow().list_obsolete_subwallet_configs()?;
        for subwalletconfig in subwalletconfigs {
            // Extract the HeritageConfig of this wallet
            self.sync_subwallet(
                subwalletconfig,
                &blockchain_factory,
                &mut obsolete_balance,
                &mut existing_utxos,
                &mut utxos_to_add,
                &mut utxos_to_delete,
                &mut txsum_to_add,
            )?;
        }

        let uptodate_balance = if let Some(current_subwallet_config) = self
            .database
            .borrow()
            .get_subwallet_config(SubwalletConfigId::Current)?
        {
            let mut balance = Balance::default();
            self.sync_subwallet(
                current_subwallet_config,
                &blockchain_factory,
                &mut balance,
                &mut existing_utxos,
                &mut utxos_to_add,
                &mut utxos_to_delete,
                &mut txsum_to_add,
            )?;
            balance
        } else {
            log::warn!("No current SubWallet to synchronize");
            Balance::default()
        };

        // Update the balance
        let new_balance = HeritageWalletBalance::new(uptodate_balance, obsolete_balance);
        log::info!("HeritageWallet::sync - new_balance={new_balance:?}");
        self.database.borrow_mut().set_balance(&new_balance)?;

        log::info!(
            "HeritageWallet::sync - utxos - remove={} add={}",
            utxos_to_delete.len(),
            utxos_to_add.len()
        );
        // Update the HeritageUtxos
        self.database.borrow_mut().delete_utxos(&utxos_to_delete)?;
        self.database.borrow_mut().add_utxos(&utxos_to_add)?;

        // Update the TransactionSummaries
        // List the existing ones
        let existing_txsum = self.database().list_transaction_summaries()?;

        // Compute the list of existing to_delete/to_add by partitioning on the presence of the TxId in txsum_to_add
        let (existing_txsum, mut existing_txsum_to_delete): (Vec<_>, Vec<_>) = existing_txsum
            .into_iter()
            .partition(|txsum| txsum_to_add.contains_key(&txsum.txid));

        // Transform the existing TxSum into a hashmap
        let existing_txsum = existing_txsum
            .into_iter()
            .map(|txsum| (txsum.txid, txsum))
            .collect::<HashMap<_, _>>();

        // We only add the TxSummary if it not present or different
        let txsum_to_add = txsum_to_add
            .into_iter()
            .filter_map(|(txid, txsum)| {
                // If we don't have it, just insert it
                if !existing_txsum.contains_key(&txid) {
                    Some(txsum)
                }
                // If we have it but it is different, we need to delete the one we have
                // Because the Database representation uses a combination of TxId and Confirmation
                // time as the key, to garantee the correct ordering of the TxSummaries
                // And most likely, what changed is the confirmation time (from None to Some)
                else if *existing_txsum.get(&txid).unwrap() != txsum {
                    existing_txsum_to_delete.push(existing_txsum.get(&txid).unwrap().clone());
                    Some(txsum)
                }
                // If we have it and it's the same, do not add it again
                else {
                    None
                }
            })
            .collect::<Vec<_>>();
        log::info!(
            "HeritageWallet::sync - tx_summaries - remove={} add={}",
            existing_txsum_to_delete.len(),
            txsum_to_add.len(),
        );
        self.database.borrow_mut().delete_transaction_summaries(
            &existing_txsum_to_delete
                .into_iter()
                .map(|txsum| (txsum.txid, txsum.confirmation_time))
                .collect(),
        )?;
        self.database
            .borrow_mut()
            .add_transaction_summaries(&txsum_to_add)?;

        // Sync FeeRate
        let fee_rate = self.sync_fee_rate(blockchain_factory)?;
        log::info!("HeritageWallet::sync - fee_rate={fee_rate:?}");

        Ok(())
    }

    fn sync_subwallet<T: BlockchainFactory>(
        &self,
        subwalletconfig: SubwalletConfig,
        blockchain_factory: &T,
        balance_acc: &mut Balance,
        existing_utxos: &mut Vec<HeritageUtxo>,
        utxos_to_add: &mut Vec<HeritageUtxo>,
        utxos_to_delete: &mut Vec<OutPoint>,
        txsum_to_add: &mut HashMap<Txid, TransactionSummary>,
    ) -> Result<()> {
        log::debug!("sync_subwallet - {subwalletconfig:?}");
        // Use the wallet first use time to limit the range of the (first) sync
        // If there is no first use, there is no need to sync either
        if subwalletconfig.subwallet_firstuse_time().is_some() {
            let subwallet = self.get_subwallet(&subwalletconfig)?;
            let sync_options = SyncOptions {
                progress: Some(Box::new(log_progress())),
            };

            blockchain_factory
                .sync_wallet(&subwallet, None, sync_options)
                .map_err(|e| Error::SyncError(e.to_string()))?;

            // Update the balance
            *balance_acc = balance_acc.clone()
                + subwallet
                    .get_balance()
                    .map_err(|e| DatabaseError::Generic(e.to_string()))?;

            // ################
            // # HeritageUtxo #
            // ################
            // Retrieve UTXOs
            let mut subwallet_utxos = subwallet
                .list_unspent()
                .map_err(|e| DatabaseError::Generic(e.to_string()))?;
            // We don't want spent unspent TX Output, whatever the fuck this means
            subwallet_utxos.retain(|lu| !lu.is_spent);
            // Extract the HeritageConfig of this wallet
            let subwallet_heritage_config = subwalletconfig.heritage_config();

            // Index HeritageUtxo for this wallet
            let mut existing_heritage_utxos = existing_utxos
                .iter()
                .filter_map(|hu| {
                    if hu.heritage_config == *subwallet_heritage_config {
                        Some((hu.outpoint, hu))
                    } else {
                        None
                    }
                })
                .collect::<HashMap<_, _>>();

            // Foreach subwallet_utxo verify if we alreay have it or not
            for subwallet_utxo in subwallet_utxos {
                if existing_heritage_utxos.contains_key(&subwallet_utxo.outpoint)
                    && existing_heritage_utxos
                        .get(&subwallet_utxo.outpoint)
                        .unwrap()
                        .confirmation_time
                        .is_some()
                {
                    // We already have it, we remove it from the set and do nothing more
                    existing_heritage_utxos.remove(&subwallet_utxo.outpoint);
                } else {
                    // We need to add this
                    let block_time = subwallet
                        .get_tx(&subwallet_utxo.outpoint.txid, false)
                        .map_err(|e| DatabaseError::Generic(e.to_string()))?
                        .expect("its present unless DB is inconsistent")
                        .confirmation_time;
                    utxos_to_add.push(HeritageUtxo {
                        outpoint: subwallet_utxo.outpoint,
                        amount: Amount::from_sat(subwallet_utxo.txout.value),
                        confirmation_time: block_time,
                        heritage_config: subwallet_heritage_config.clone(),
                    });
                }
            }

            // Stop the borrow on existing_utxos by releasing the references on its content
            let existing_heritage_utxos =
                existing_heritage_utxos.into_keys().collect::<HashSet<_>>();

            // Remove those element from existing_utxos
            existing_utxos.retain(|hu| !existing_heritage_utxos.contains(&hu.outpoint));

            // At this point existing_heritage_utxos contains only OutPoint of HeritageUtxo that are no longer valid.
            // We add them for removal
            utxos_to_delete.append(&mut existing_heritage_utxos.into_iter().collect());

            // ######################
            // # TransactionSummary #
            // ######################
            // Retrieve the subwallet tx
            let subwallet_txs = subwallet
                .list_transactions(false)
                .map_err(|e| DatabaseError::Generic(e.to_string()))?;
            for subwallet_tx in subwallet_txs {
                txsum_to_add
                    .entry(subwallet_tx.txid)
                    .and_modify(|tx_sum| {
                        tx_sum.received += Amount::from_sat(subwallet_tx.received);
                        tx_sum.sent += Amount::from_sat(subwallet_tx.sent);
                        if let Some(fee) = subwallet_tx.fee {
                            tx_sum.fee = Amount::from_sat(fee);
                        }
                    })
                    .or_insert(TransactionSummary {
                        txid: subwallet_tx.txid,
                        confirmation_time: subwallet_tx.confirmation_time,
                        received: Amount::from_sat(subwallet_tx.received),
                        sent: Amount::from_sat(subwallet_tx.sent),
                        fee: Amount::from_sat(subwallet_tx.fee.unwrap_or(0)),
                    });
            }
        } else {
            log::info!(
                "Skipping sync of SubwalletConfig Id={} because it was never used",
                subwalletconfig.subwallet_id()
            )
        }
        Ok(())
    }

    fn sync_fee_rate<T: BlockchainFactory>(&self, blockchain_factory: T) -> Result<FeeRate> {
        log::debug!("HeritageWallet::sync_fee_rate");
        let block_inclusion_objective = self.get_block_inclusion_objective()?;
        log::debug!(
            "HeritageWallet::sync_fee_rate - block_inclusion_objective={block_inclusion_objective}"
        );

        // The RPC method "estimatesmartfee" returns a result in BTC/kvB
        let fee_rate = blockchain_factory
            .build("unimportant", None)
            .map_err(|e| Error::BlockchainProviderError(e.to_string()))?
            .estimate_fee(block_inclusion_objective.0 as usize)
            .map_err(|e| Error::BlockchainProviderError(e.to_string()))?;

        self.database.borrow_mut().set_fee_rate(&fee_rate)?;
        Ok(fee_rate)
    }
}
