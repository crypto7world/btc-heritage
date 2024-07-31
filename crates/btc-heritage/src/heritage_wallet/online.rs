use std::collections::{HashMap, HashSet};

use bdk::{
    blockchain::{log_progress, Blockchain, BlockchainFactory},
    database::Database,
    Balance, SyncOptions,
};

use super::{
    HeritageUtxo, HeritageWallet, HeritageWalletBalance, SubwalletConfigId, TransactionSummary,
};
use crate::{
    bitcoin::{Amount, FeeRate, OutPoint, Txid},
    database::TransacHeritageDatabase,
    errors::{DatabaseError, Error, Result},
    heritage_wallet::TransactionSummaryOwnedIO,
    subwallet_config::SubwalletConfig,
    utils::sort_transaction_details_with_raw,
};

impl<D: TransacHeritageDatabase> HeritageWallet<D> {
    pub fn sync<T: BlockchainFactory>(&self, blockchain_factory: T) -> Result<()> {
        log::debug!("HeritageWallet::sync");
        // This cache will serve to build the TransactionSummary list
        // /!\ It is crucial that it is filled from oldest to newest so that we can
        // use it in one-pass. Each time we search this cache for an owned-Outpoint
        // we expect it to be in there if it exists.
        let mut tx_owned_io_cache: HashMap<OutPoint, TransactionSummaryOwnedIO> = HashMap::new();
        // Manage the HeritageUtxo updates
        let mut existing_utxos = self.database().list_utxos()?;
        let mut utxos_to_add = vec![];
        let mut utxos_to_delete = vec![];
        // Manage the TransactionSummary updates
        let mut txsum_to_add = HashMap::new();
        // Start obsolete_balance at zero
        let mut obsolete_balance = Balance::default();
        // Walk over every subwallets and sync them
        let mut subwalletconfigs = self.database.borrow().list_obsolete_subwallet_configs()?;
        // Make sure the obsolete_subwallet_configs are in order
        subwalletconfigs.sort_by_key(|swc| {
            swc.subwallet_firstuse_time()
                .expect("obsolete subwallet have always been used")
        });
        for subwalletconfig in subwalletconfigs {
            // Extract the HeritageConfig of this wallet
            self.sync_subwallet(
                subwalletconfig,
                &blockchain_factory,
                &mut tx_owned_io_cache,
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
                &mut tx_owned_io_cache,
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
        tx_owned_io_cache: &mut HashMap<OutPoint, TransactionSummaryOwnedIO>,
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
                        address: crate::bitcoin::Address::from_script(
                            subwallet_utxo.txout.script_pubkey.as_script(),
                            *crate::utils::bitcoin_network_from_env(),
                        )
                        .expect("script should always be valid")
                        .into(),
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
            let mut subwallet_txs = subwallet
                .list_transactions(true)
                .map_err(|e| DatabaseError::Generic(e.to_string()))?;
            sort_transaction_details_with_raw(&mut subwallet_txs);

            // Retrieve the subwallet scriptpubkeys
            let subwallet_spks = subwallet
                .database()
                .iter_script_pubkeys(None)
                .map_err(|e| DatabaseError::Generic(e.to_string()))?
                .into_iter()
                .collect::<HashSet<_>>();
            for subwallet_tx in subwallet_txs {
                let raw_tx = subwallet_tx
                    .transaction
                    .expect("we asked it to be included");
                // Compose the set of "parent TXs"
                let parent_txids = raw_tx
                    .input
                    .iter()
                    .map(|txin| txin.previous_output.txid)
                    .collect();

                // Process the Outputs to verify if they are owned
                // Update the cache as we construct the owned_outputs
                let mut owned_outputs = (0u32..)
                    .zip(raw_tx.output.into_iter())
                    .filter(|(_, o)| subwallet_spks.contains(&o.script_pubkey))
                    .map(|(i, o)| {
                        let tsoio = TransactionSummaryOwnedIO(
                            (&o.script_pubkey).try_into().expect("comes from DB"),
                            Amount::from_sat(o.value),
                        );
                        let outpoint = OutPoint {
                            txid: subwallet_tx.txid,
                            vout: i,
                        };
                        tx_owned_io_cache.insert(outpoint, tsoio.clone());
                        tsoio
                    })
                    .collect::<Vec<_>>();

                // Process the Inputs to verify if they are owned
                let mut owned_inputs = raw_tx
                    .input
                    .into_iter()
                    // Remove is appropriate because a BTC UTXO can only be consummed once
                    // So if we match, we might as well remove the match from the cache
                    // + it is neat because we don't have to clone and it fits naturally in filter_map
                    .filter_map(|i| tx_owned_io_cache.remove(&i.previous_output))
                    .collect::<Vec<_>>();

                txsum_to_add
                    .entry(subwallet_tx.txid)
                    .and_modify(|tx_sum| {
                        tx_sum.owned_inputs.append(&mut owned_inputs);
                        tx_sum.owned_outputs.append(&mut owned_outputs);
                        if let Some(fee) = subwallet_tx.fee {
                            tx_sum.fee = Amount::from_sat(fee);
                        }
                    })
                    .or_insert(TransactionSummary {
                        txid: subwallet_tx.txid,
                        confirmation_time: subwallet_tx.confirmation_time,
                        owned_inputs,
                        owned_outputs,
                        fee: Amount::from_sat(subwallet_tx.fee.unwrap_or(0)),
                        parent_txids,
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
        let bdk_fee_rate = blockchain_factory
            .build("unimportant", None)
            .map_err(|e| Error::BlockchainProviderError(e.to_string()))?
            .estimate_fee(block_inclusion_objective.0 as usize)
            .map_err(|e| Error::BlockchainProviderError(e.to_string()))?;

        let fee_rate = FeeRate::from_sat_per_vb_unchecked(bdk_fee_rate.as_sat_per_vb() as u64);
        self.database.borrow_mut().set_fee_rate(&fee_rate)?;
        Ok(fee_rate)
    }
}
