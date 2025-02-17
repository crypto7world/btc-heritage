use std::collections::HashSet;

use btc_heritage::{
    bdk_types,
    bitcoin::{FeeRate, OutPoint, Txid},
    database::{
        paginate::{ContinuationToken, Paginated},
        HeritageDatabase, TransacHeritageDatabase, TransacHeritageOperation,
    },
    errors::DatabaseError,
    heritage_wallet::{HeritageUtxo, SubwalletConfigId, TransactionSummary},
    subwallet_config::SubwalletConfig,
    AccountXPub, BlockInclusionObjective, HeritageWalletBalance,
};

use super::{HeritageWalletDatabase, KeyMapper};
type Result<T> = core::result::Result<T, DatabaseError>;

#[derive(Debug)]
pub struct HeritageWalletDatabaseTransaction {
    inner: super::super::DatabaseTransaction,
    errors_if_fail: Vec<DatabaseError>,
    prefix: String,
}
impl HeritageWalletDatabaseTransaction {
    fn key(&self, key_mapper: &KeyMapper) -> String {
        key_mapper.key(&self.prefix)
    }
}

impl TransacHeritageOperation for HeritageWalletDatabaseTransaction {
    fn put_subwallet_config(
        &mut self,
        index: SubwalletConfigId,
        subwallet_config: &SubwalletConfig,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabaseTransaction::put_subwallet_config - index={index:?} subwallet_config={subwallet_config:?}");
        let key = self.key(&KeyMapper::SubwalletConfig(Some(index)));
        self.inner.put_item(&key, subwallet_config)?;
        self.errors_if_fail
            .push(DatabaseError::SubwalletConfigAlreadyExist(index));
        Ok(())
    }

    fn safe_update_current_subwallet_config(
        &mut self,
        new_subwallet_config: &SubwalletConfig,
        old_subwallet_config: Option<&SubwalletConfig>,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabaseTransaction::safe_update_current_subwallet_config - new_subwallet_config={new_subwallet_config:?} old_subwallet_config={old_subwallet_config:?}");
        let key = self.key(&KeyMapper::SubwalletConfig(Some(
            SubwalletConfigId::Current,
        )));

        self.inner
            .compare_and_swap(&key, old_subwallet_config, Some(new_subwallet_config))?;

        self.errors_if_fail
            .push(DatabaseError::UnexpectedCurrentSubwalletConfig);
        Ok(())
    }

    fn delete_unused_account_xpub(&mut self, account_xpub: &AccountXPub) -> Result<()> {
        log::debug!("HeritageWalletDatabaseTransaction::delete_unused_account_xpub - account_xpub={account_xpub:?}");
        let key = self.key(&KeyMapper::UnusedAccountXPub(Some(
            account_xpub.descriptor_id(),
        )));

        self.inner
            .compare_and_swap(&key, Some(account_xpub), None)?;

        self.errors_if_fail
            .push(DatabaseError::AccountXPubInexistant(
                account_xpub.descriptor_id(),
            ));
        Ok(())
    }
}

impl TransacHeritageOperation for HeritageWalletDatabase {
    fn put_subwallet_config(
        &mut self,
        index: SubwalletConfigId,
        subwallet_config: &SubwalletConfig,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabase::put_subwallet_config - index={index:?} subwallet_config={subwallet_config:?}");
        let key = self.key(&KeyMapper::SubwalletConfig(Some(index)));
        self.db
            ._put_item(&key, subwallet_config)
            .map_err(|e| match e {
                crate::database::errors::DbError::KeyAlreadyExists(_) => {
                    DatabaseError::SubwalletConfigAlreadyExist(index)
                }
                _ => e.into(),
            })?;
        Ok(())
    }

    fn safe_update_current_subwallet_config(
        &mut self,
        new_subwallet_config: &SubwalletConfig,
        old_subwallet_config: Option<&SubwalletConfig>,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabase::safe_update_current_subwallet_config - new_subwallet_config={new_subwallet_config:?} old_subwallet_config={old_subwallet_config:?}");
        let key = self.key(&KeyMapper::SubwalletConfig(Some(
            SubwalletConfigId::Current,
        )));
        self.db
            ._compare_and_swap(&key, old_subwallet_config, Some(new_subwallet_config))
            .map_err(|e| match e {
                crate::database::errors::DbError::CompareAndSwapError(_) => {
                    DatabaseError::UnexpectedCurrentSubwalletConfig
                }
                _ => e.into(),
            })?;
        Ok(())
    }

    fn delete_unused_account_xpub(&mut self, account_xpub: &AccountXPub) -> Result<()> {
        log::debug!(
            "HeritageWalletDatabase::delete_unused_account_xpub - account_xpub={account_xpub:?}"
        );
        let key = self.key(&KeyMapper::UnusedAccountXPub(Some(
            account_xpub.descriptor_id(),
        )));

        self.db
            ._compare_and_swap(&key, Some(account_xpub), None)
            .map_err(|e| match e {
                crate::database::errors::DbError::CompareAndSwapError(_) => {
                    DatabaseError::AccountXPubInexistant(account_xpub.descriptor_id())
                }
                _ => e.into(),
            })?;
        Ok(())
    }
}

impl TransacHeritageDatabase for HeritageWalletDatabase {
    type Transac = HeritageWalletDatabaseTransaction;

    fn begin_transac(&self) -> Self::Transac {
        HeritageWalletDatabaseTransaction {
            inner: self.db._begin_transac(),
            errors_if_fail: vec![],
            prefix: self.prefix.clone(),
        }
    }

    fn commit_transac(&mut self, transac: Self::Transac) -> Result<()> {
        let HeritageWalletDatabaseTransaction {
            inner: transac,
            mut errors_if_fail,
            ..
        } = transac;
        self.db._commit_transac(transac).map_err(|e| match &e {
            crate::database::errors::DbError::TransactionFailed { idx, .. } => {
                log::error!("{e}");
                errors_if_fail.remove(*idx)
            }
            _ => DatabaseError::Generic(e.to_string()),
        })
    }
}

impl HeritageDatabase for HeritageWalletDatabase {
    fn get_subwallet_config(&self, index: SubwalletConfigId) -> Result<Option<SubwalletConfig>> {
        log::debug!("HeritageWalletDatabase::get_subwallet_config - index={index:?}");
        let key = self.key(&KeyMapper::SubwalletConfig(Some(index)));
        Ok(self.db._get_item(&key)?)
    }

    fn list_obsolete_subwallet_configs(&self) -> Result<Vec<SubwalletConfig>> {
        log::debug!("HeritageWalletDatabase::list_obsolete_subwallet_configs");
        let prefix = self.key(&KeyMapper::SubwalletConfig(None)) + "a";
        Ok(self.db._query(&prefix)?)
    }

    fn get_unused_account_xpub(&self) -> Result<Option<AccountXPub>> {
        log::debug!("HeritageWalletDatabase::get_unused_account_xpub");
        let prefix = self.key(&KeyMapper::UnusedAccountXPub(None));
        Ok(self.db._query(&prefix)?.into_iter().next())
    }

    fn list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageWalletDatabase::list_unused_account_xpubs");
        let prefix = self.key(&KeyMapper::UnusedAccountXPub(None));
        Ok(self.db._query(&prefix)?)
    }

    fn list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageWalletDatabase::list_used_account_xpubs");
        let prefix = self.key(&KeyMapper::SubwalletConfig(None));
        let swcs: Vec<SubwalletConfig> = self.db._query(&prefix)?;
        Ok(swcs.into_iter().map(|swc| swc.into_parts().0).collect())
    }

    fn add_unused_account_xpubs(&mut self, account_xpubs: &Vec<AccountXPub>) -> Result<()> {
        log::debug!(
            "HeritageWalletDatabase::add_unused_account_xpubs - account_xpubs={account_xpubs:?}"
        );

        // Retrieve the existing and used Account XPubs
        let used_account_xpubs_index = self
            .list_used_account_xpubs()?
            .into_iter()
            .map(|ad| ad.descriptor_id())
            .collect::<HashSet<_>>();

        // Actually process the Account XPubs
        let descriptors_to_add = account_xpubs
            .into_iter()
            .filter(|ad| {
                let id = ad.descriptor_id();
                if used_account_xpubs_index.contains(&id){
                    log::warn!("HeritageWalletDatabase::add_unused_account_xpubs - Ignoring account_xpub because we already used it: {ad:?}");
                    false
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();
        if descriptors_to_add.len() > 0 {
            let mut txn = self.db._begin_transac();

            for descriptor in descriptors_to_add {
                txn.update_item(
                    &self.key(&KeyMapper::UnusedAccountXPub(Some(
                        descriptor.descriptor_id(),
                    ))),
                    descriptor,
                )?;
            }
            self.db._commit_transac(txn)?;
        }
        Ok(())
    }

    fn add_utxos(&mut self, utxos: &Vec<HeritageUtxo>) -> Result<()> {
        log::debug!("HeritageWalletDatabase::add_utxos - utxos={utxos:?}");
        if utxos.len() > 0 {
            let mut txn = self.db._begin_transac();

            for utxo in utxos {
                txn.update_item(
                    &self.key(&KeyMapper::HeritageUtxo(Some(&utxo.outpoint))),
                    utxo,
                )?;
            }
            self.db._commit_transac(txn)?;
        }
        Ok(())
    }

    fn delete_utxos(&mut self, outpoints: &Vec<OutPoint>) -> Result<()> {
        log::debug!("HeritageWalletDatabase::delete_utxos - outpoints={outpoints:?}");
        if outpoints.len() > 0 {
            let mut txn = self.db._begin_transac();

            for outpoint in outpoints {
                txn.delete_item(&self.key(&KeyMapper::HeritageUtxo(Some(outpoint))));
            }
            self.db._commit_transac(txn)?;
        }
        Ok(())
    }

    fn list_utxos(&self) -> Result<Vec<HeritageUtxo>> {
        log::debug!("HeritageWalletDatabase::list_utxos");
        let prefix = self.key(&KeyMapper::HeritageUtxo(None));
        Ok(self.db._query(&prefix)?)
    }

    fn paginate_utxos(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<HeritageUtxo>> {
        log::debug!("HeritageWalletDatabase::paginate_utxos - page_size={page_size} continuation_token={continuation_token:?}");
        let prefix = self.key(&KeyMapper::HeritageUtxo(None));

        let (page, next_key) =
            self.db
                ._query_page(&prefix, page_size, continuation_token.map(|ct| ct.0))?;
        let continuation_token = next_key.map(|s| ContinuationToken(s));
        let page = Paginated {
            page,
            continuation_token,
        };
        Ok(page)
    }

    fn add_transaction_summaries(
        &mut self,
        transaction_summaries: &Vec<TransactionSummary>,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabase::add_transaction_summaries - transaction_summaries={transaction_summaries:?}");
        if transaction_summaries.len() > 0 {
            let mut txn = self.db._begin_transac();

            for transaction_summary in transaction_summaries {
                txn.update_item(
                    &self.key(&KeyMapper::TxSummary(Some((
                        &transaction_summary.txid,
                        transaction_summary.confirmation_time.as_ref(),
                    )))),
                    transaction_summary,
                )?;
            }
            self.db._commit_transac(txn)?;
        }
        Ok(())
    }

    fn delete_transaction_summaries(
        &mut self,
        key_to_delete: &Vec<(Txid, Option<bdk_types::BlockTime>)>,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabase::delete_transaction_summaries - key_to_delete={key_to_delete:?}");
        if key_to_delete.len() > 0 {
            let mut txn = self.db._begin_transac();

            for (txid, confirmation_time) in key_to_delete {
                txn.delete_item(&self.key(&KeyMapper::TxSummary(Some((
                    txid,
                    confirmation_time.as_ref(),
                )))));
            }
            self.db._commit_transac(txn)?;
        }
        Ok(())
    }

    fn list_transaction_summaries(&self) -> Result<Vec<TransactionSummary>> {
        log::debug!("HeritageWalletDatabase::list_transaction_summaries");
        let prefix = self.key(&KeyMapper::TxSummary(None));
        Ok(self.db._query_rev(&prefix)?)
    }

    fn paginate_transaction_summaries(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<TransactionSummary>> {
        log::debug!("HeritageWalletDatabase::paginate_transaction_summaries - page_size={page_size} continuation_token={continuation_token:?}");
        let prefix = self.key(&KeyMapper::TxSummary(None));
        let (page, next_key) =
            self.db
                ._query_page_rev(&prefix, page_size, continuation_token.map(|ct| ct.0))?;
        let continuation_token = next_key.map(|s| ContinuationToken(s));
        let page = Paginated {
            page,
            continuation_token,
        };
        Ok(page)
    }

    fn get_balance(&self) -> Result<Option<HeritageWalletBalance>> {
        log::debug!("HeritageWalletDatabase::get_balance");
        let key = self.key(&KeyMapper::WalletBalance);
        Ok(self.db._get_item(&key)?)
    }

    fn set_balance(&mut self, new_balance: &HeritageWalletBalance) -> Result<()> {
        log::debug!("HeritageWalletDatabase::get_balance");
        let key = self.key(&KeyMapper::WalletBalance);
        self.db._update_item(&key, new_balance)?;
        Ok(())
    }

    fn get_fee_rate(&self) -> Result<Option<FeeRate>> {
        log::debug!("HeritageWalletDatabase::get_fee_rate");
        let key = self.key(&KeyMapper::FeeRate);
        Ok(self.db._get_item(&key)?)
    }

    fn set_fee_rate(&mut self, new_fee_rate: &FeeRate) -> Result<()> {
        log::debug!("HeritageWalletDatabase::set_fee_rate - new_fee_rate={new_fee_rate:?}");
        let key = self.key(&KeyMapper::FeeRate);
        self.db._update_item(&key, new_fee_rate)?;
        Ok(())
    }

    fn get_block_inclusion_objective(&self) -> Result<Option<BlockInclusionObjective>> {
        log::debug!("HeritageWalletDatabase::get_block_inclusion_objective");
        let key = self.key(&KeyMapper::BlockInclusionObjective);
        Ok(self.db._get_item(&key)?)
    }

    fn set_block_inclusion_objective(
        &mut self,
        new_objective: BlockInclusionObjective,
    ) -> Result<()> {
        log::debug!("HeritageWalletDatabase::set_block_inclusion_objective - new_objective={new_objective:?}");
        let key = self.key(&KeyMapper::BlockInclusionObjective);
        self.db._update_item(&key, &new_objective)?;
        Ok(())
    }
}
