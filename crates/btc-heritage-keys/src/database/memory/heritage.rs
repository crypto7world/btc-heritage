use std::{
    any::Any,
    collections::{BTreeMap, HashSet},
    ops::{Bound, Deref, DerefMut},
    option::Option,
};

use bdk::{
    bitcoin::{OutPoint, Txid},
    BlockTime, FeeRate,
};

use crate::{
    accountxpub::AccountXPubId,
    database::{
        paginate::{ContinuationToken, Paginated},
        HeritageDatabase, TransacHeritageDatabase, TransacHeritageOperation,
    },
    errors::DatabaseError,
    heritagewallet::{
        BlockInclusionObjective, HeritageUtxo, HeritageWalletBalance, SubwalletConfigId,
        TransactionSummary,
    },
    subwalletconfig::SubwalletConfig,
    AccountXPub,
};

use super::{HeritageMemoryDatabase, HeritageMonoItemKeyMapper, Result};

#[derive(Debug)]
enum TransacOp {
    PutSubwalletConfig(SubwalletConfigId, SubwalletConfig),
    SafeUpdateCurrentSubwalletConfig(SubwalletConfig, Option<SubwalletConfig>),
    DeleteUnusedAccountXPub(AccountXPubId),
}
impl TransacOp {
    fn condition_check(&self, table: &BTreeMap<String, Box<dyn Any + Send + Sync>>) -> bool {
        match self {
            TransacOp::PutSubwalletConfig(index, _) => {
                let key = HeritageMonoItemKeyMapper::WalletConfig(Some(*index)).key();
                !table.contains_key(&key)
            }
            TransacOp::SafeUpdateCurrentSubwalletConfig(_, old_subwallet_config) => {
                let key =
                    HeritageMonoItemKeyMapper::WalletConfig(Some(SubwalletConfigId::Current)).key();
                let database_old_subwallet_config = table.get(&key).map(|b| {
                    b.downcast_ref::<SubwalletConfig>()
                        .expect("This is a SubwalletConfig")
                });
                if let Some(old_subwallet_config) = old_subwallet_config {
                    if let Some(database_old_subwallet_config) = database_old_subwallet_config {
                        return database_old_subwallet_config.subwallet_id()
                            == old_subwallet_config.subwallet_id()
                            && database_old_subwallet_config.ext_descriptor()
                                == old_subwallet_config.ext_descriptor()
                            && database_old_subwallet_config.change_descriptor()
                                == old_subwallet_config.change_descriptor()
                            && database_old_subwallet_config.account_xpub()
                                == old_subwallet_config.account_xpub()
                            && database_old_subwallet_config.subwallet_firstuse_time()
                                == old_subwallet_config.subwallet_firstuse_time();
                    }
                } else {
                    return database_old_subwallet_config.is_none();
                }
                false
            }
            TransacOp::DeleteUnusedAccountXPub(account_xpub_id) => {
                let key =
                    HeritageMonoItemKeyMapper::UnusedAccountXPub(Some(*account_xpub_id)).key();
                table.contains_key(&key)
            }
        }
    }
    fn do_op(self, table: &mut BTreeMap<String, Box<dyn Any + Send + Sync>>) {
        match self {
            TransacOp::PutSubwalletConfig(index, subwallet_config) => {
                let key = HeritageMonoItemKeyMapper::WalletConfig(Some(index)).key();
                table.insert(key, Box::new(subwallet_config));
            }
            TransacOp::SafeUpdateCurrentSubwalletConfig(new_subwallet_config, _) => {
                let key =
                    HeritageMonoItemKeyMapper::WalletConfig(Some(SubwalletConfigId::Current)).key();
                table.insert(key, Box::new(new_subwallet_config));
            }
            TransacOp::DeleteUnusedAccountXPub(account_xpub_id) => {
                let key = HeritageMonoItemKeyMapper::UnusedAccountXPub(Some(account_xpub_id)).key();
                table.remove(&key);
            }
        }
    }
}

#[derive(Debug)]
pub struct HeritageMemoryDatabaseTransac(Vec<TransacOp>);

impl TransacHeritageOperation for HeritageMemoryDatabaseTransac {
    fn put_subwallet_config(
        &mut self,
        index: SubwalletConfigId,
        subwallet_config: &SubwalletConfig,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabaseTransac::put_subwallet_config - index={index:?} subwallet_config={subwallet_config:?}");
        self.0.push(TransacOp::PutSubwalletConfig(
            index,
            subwallet_config.clone(),
        ));
        Ok(())
    }

    fn safe_update_current_subwallet_config(
        &mut self,
        new_subwallet_config: &SubwalletConfig,
        old_subwallet_config: Option<&SubwalletConfig>,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabaseTransac::safe_update_current_subwallet_config - new_subwallet_config={new_subwallet_config:?} old_subwallet_config={old_subwallet_config:?}");
        self.0.push(TransacOp::SafeUpdateCurrentSubwalletConfig(
            new_subwallet_config.clone(),
            old_subwallet_config.cloned(),
        ));
        Ok(())
    }

    fn delete_unused_account_xpub(&mut self, account_xpub: &AccountXPub) -> Result<()> {
        log::debug!("HeritageMemoryDatabaseTransac::delete_unused_account_xpub - account_xpub={account_xpub:?}");
        self.0.push(TransacOp::DeleteUnusedAccountXPub(
            account_xpub.descriptor_id(),
        ));
        Ok(())
    }
}

impl TransacHeritageOperation for HeritageMemoryDatabase {
    fn put_subwallet_config(
        &mut self,
        index: SubwalletConfigId,
        subwallet_config: &SubwalletConfig,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::put_subwallet_config - index={index:?} subwallet_config={subwallet_config:?}");
        let op = TransacOp::PutSubwalletConfig(index, subwallet_config.clone());
        let mut table = self.table.write().unwrap();
        if !op.condition_check(table.deref()) {
            return Err(DatabaseError::SubwalletConfigAlreadyExist(index));
        }
        op.do_op(table.deref_mut());
        Ok(())
    }

    fn safe_update_current_subwallet_config(
        &mut self,
        new_subwallet_config: &SubwalletConfig,
        old_subwallet_config: Option<&SubwalletConfig>,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::safe_update_current_subwallet_config - new_subwallet_config={new_subwallet_config:?} old_subwallet_config={old_subwallet_config:?}");
        let op = TransacOp::SafeUpdateCurrentSubwalletConfig(
            new_subwallet_config.clone(),
            old_subwallet_config.cloned(),
        );
        let mut table = self.table.write().unwrap();
        if !op.condition_check(table.deref()) {
            return Err(DatabaseError::UnexpectedCurrentSubwalletConfig);
        }
        op.do_op(table.deref_mut());
        Ok(())
    }

    fn delete_unused_account_xpub(&mut self, account_xpub: &AccountXPub) -> Result<()> {
        log::debug!(
            "HeritageMemoryDatabase::delete_unused_account_xpub - account_xpub={account_xpub:?}"
        );
        let op = TransacOp::DeleteUnusedAccountXPub(account_xpub.descriptor_id());
        let mut table = self.table.write().unwrap();
        if !op.condition_check(table.deref()) {
            return Err(DatabaseError::AccountXPubInexistant(
                account_xpub.descriptor_id(),
            ));
        }
        op.do_op(table.deref_mut());
        Ok(())
    }
}

impl TransacHeritageDatabase for HeritageMemoryDatabase {
    type Transac = HeritageMemoryDatabaseTransac;

    fn begin_transac(&self) -> Self::Transac {
        log::debug!("HeritageMemoryDatabase::begin_transac");
        HeritageMemoryDatabaseTransac(Vec::new())
    }

    fn commit_transac(&mut self, transac: Self::Transac) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::commit_transac - transac={transac:?}");
        let mut table = self.table.write().unwrap();
        for op in &transac.0 {
            if !op.condition_check(table.deref()) {
                return Err(match op {
                    TransacOp::PutSubwalletConfig(id, _) => {
                        DatabaseError::SubwalletConfigAlreadyExist(*id)
                    }
                    TransacOp::SafeUpdateCurrentSubwalletConfig(_, _) => {
                        DatabaseError::UnexpectedCurrentSubwalletConfig
                    }
                    TransacOp::DeleteUnusedAccountXPub(xpubid) => {
                        DatabaseError::AccountXPubInexistant(*xpubid)
                    }
                });
            }
        }
        for op in transac.0 {
            op.do_op(table.deref_mut());
        }
        Ok(())
    }
}

impl HeritageDatabase for HeritageMemoryDatabase {
    fn get_subwallet_config(&self, index: SubwalletConfigId) -> Result<Option<SubwalletConfig>> {
        log::debug!("HeritageMemoryDatabase::get_subwallet_config - index={index:?}");
        let key = HeritageMonoItemKeyMapper::WalletConfig(Some(index)).key();
        Ok(self.table.read().unwrap().get(&key).map(|b| {
            b.downcast_ref::<SubwalletConfig>()
                .expect("this is a SubwalletConfig")
                .clone()
        }))
    }

    fn list_obsolete_subwallet_configs(&self) -> Result<Vec<SubwalletConfig>> {
        log::debug!("HeritageMemoryDatabase::list_obsolete_subwallet_configs");
        let key = HeritageMonoItemKeyMapper::WalletConfig(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Excluded(key + ":");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .map(|(_, b)| {
                b.downcast_ref::<SubwalletConfig>()
                    .expect("this is a SubwalletConfig")
                    .clone()
            })
            .collect())
    }

    fn get_unused_account_xpub(&self) -> Result<Option<AccountXPub>> {
        log::debug!("HeritageMemoryDatabase::get_unused_account_xpub");
        let key = HeritageMonoItemKeyMapper::UnusedAccountXPub(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Excluded(key + ":");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .take(1)
            .map(|(_, b)| {
                b.downcast_ref::<AccountXPub>()
                    .expect("this is a AccountXPub")
                    .clone()
            })
            .next())
    }

    fn list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageMemoryDatabase::list_unused_account_xpubs");
        let key = HeritageMonoItemKeyMapper::UnusedAccountXPub(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Excluded(key + ":");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .map(|(_, b)| {
                b.downcast_ref::<AccountXPub>()
                    .expect("this is a AccountXPub")
                    .clone()
            })
            .collect())
    }

    fn list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>> {
        log::debug!("HeritageMemoryDatabase::list_used_account_xpubs");
        let key = HeritageMonoItemKeyMapper::WalletConfig(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Excluded(key + "{");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .map(|(_, b)| {
                b.downcast_ref::<SubwalletConfig>()
                    .expect("this is a SubwalletConfig")
                    .account_xpub()
                    .clone()
            })
            .collect())
    }

    fn add_unused_account_xpubs(&mut self, account_xpubs: &Vec<AccountXPub>) -> Result<()> {
        log::debug!(
            "HeritageMemoryDatabase::add_unused_account_xpubs - account_xpubs={account_xpubs:?}"
        );
        let used_account_xpubs_index = self
            .list_used_account_xpubs()?
            .into_iter()
            .map(|ad| ad.descriptor_id())
            .collect::<HashSet<_>>();
        let descriptors_to_add = account_xpubs
            .into_iter()
            .filter(|ad| {
                let id = ad.descriptor_id();
                if used_account_xpubs_index.contains(&id){
                    log::warn!("HeritageMemoryDatabase::add_unused_account_xpubs - Ignoring account_xpub because we already used it: {ad:?}");
                    false
                } else {
                    true
                }
            })
            .collect::<Vec<_>>();
        if descriptors_to_add.len() > 0 {
            let mut table = self.table.write().unwrap();
            for descriptor in descriptors_to_add {
                let key =
                    HeritageMonoItemKeyMapper::UnusedAccountXPub(Some(descriptor.descriptor_id()))
                        .key();
                table.insert(key, Box::new(descriptor.clone()));
            }
        }
        Ok(())
    }

    fn add_utxos(&mut self, utxos: &Vec<HeritageUtxo>) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::add_utxos - utxos={utxos:?}");
        let mut table = self.table.write().unwrap();
        for utxo in utxos {
            let key = HeritageMonoItemKeyMapper::HeritageUtxo(Some(&utxo.outpoint)).key();
            table.insert(key, Box::new(utxo.clone()));
        }
        Ok(())
    }

    fn delete_utxos(&mut self, outpoints: &Vec<OutPoint>) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::delete_utxos - outpoints={outpoints:?}");
        let mut table = self.table.write().unwrap();
        for outpoint in outpoints {
            let key = HeritageMonoItemKeyMapper::HeritageUtxo(Some(outpoint)).key();
            table.remove(&key);
        }
        Ok(())
    }

    fn list_utxos(&self) -> Result<Vec<HeritageUtxo>> {
        log::debug!("HeritageMemoryDatabase::list_utxos");
        let key = HeritageMonoItemKeyMapper::HeritageUtxo(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Excluded(key + "{");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .map(|(_, b)| {
                b.downcast_ref::<HeritageUtxo>()
                    .expect("this is an HeritageUtxo")
                    .clone()
            })
            .collect())
    }

    fn paginate_utxos(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<HeritageUtxo>> {
        log::debug!("HeritageMemoryDatabase::paginate_utxos - page_size={page_size} continuation_token={continuation_token:?}");
        let key = HeritageMonoItemKeyMapper::HeritageUtxo(None).key();
        let lower_bound = if let Some(token) = continuation_token {
            Bound::Included(token.0)
        } else {
            Bound::Included(key.clone() + "0")
        };
        let upper_bound = Bound::Excluded(key + "{");
        let mut page = self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .take(page_size + 1)
            .map(|(_, b)| {
                b.downcast_ref::<HeritageUtxo>()
                    .expect("this is an HeritageUtxo")
                    .clone()
            })
            .collect::<Vec<_>>();
        let continuation_token = if page.len() > page_size {
            let k =
                HeritageMonoItemKeyMapper::HeritageUtxo(Some(&page.pop().unwrap().outpoint)).key();
            Some(ContinuationToken(k))
        } else {
            None
        };

        Ok(Paginated {
            page,
            continuation_token,
        })
    }

    fn add_transaction_summaries(
        &mut self,
        transaction_summaries: &Vec<TransactionSummary>,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::add_transaction_summaries - transaction_summaries={transaction_summaries:?}");
        let mut table = self.table.write().unwrap();
        for transaction_summary in transaction_summaries {
            let key = HeritageMonoItemKeyMapper::TxSummary(Some((
                &transaction_summary.txid,
                transaction_summary.confirmation_time.as_ref(),
            )))
            .key();
            table.insert(key, Box::new(transaction_summary.clone()));
        }
        Ok(())
    }

    fn delete_transaction_summaries(
        &mut self,
        key_to_delete: &Vec<(Txid, Option<BlockTime>)>,
    ) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::delete_transaction_summaries - key_to_delete={key_to_delete:?}");
        let mut table = self.table.write().unwrap();
        for (txid, confirmation_time) in key_to_delete {
            let key =
                HeritageMonoItemKeyMapper::TxSummary(Some((txid, confirmation_time.as_ref())))
                    .key();
            table.remove(&key);
        }
        Ok(())
    }

    fn list_transaction_summaries(&self) -> Result<Vec<TransactionSummary>> {
        log::debug!("HeritageMemoryDatabase::list_transaction_summaries");
        let key = HeritageMonoItemKeyMapper::TxSummary(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = Bound::Included(key + "9");
        Ok(self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .rev()
            .map(|(_, b)| {
                b.downcast_ref::<TransactionSummary>()
                    .expect("this is a TransactionSummary")
                    .clone()
            })
            .collect())
    }

    fn paginate_transaction_summaries(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<TransactionSummary>> {
        log::debug!("HeritageMemoryDatabase::paginate_transaction_summaries - page_size={page_size} continuation_token={continuation_token:?}");
        let key = HeritageMonoItemKeyMapper::TxSummary(None).key();
        let lower_bound = Bound::Included(key.clone() + "0");
        let upper_bound = if let Some(token) = continuation_token {
            Bound::Included(token.0)
        } else {
            Bound::Included(key + "9")
        };

        let mut page = self
            .table
            .read()
            .unwrap()
            .range((lower_bound, upper_bound))
            .rev()
            .take(page_size + 1)
            .map(|(_, b)| {
                b.downcast_ref::<TransactionSummary>()
                    .expect("this is a TransactionSummary")
                    .clone()
            })
            .collect::<Vec<_>>();
        let continuation_token = if page.len() > page_size {
            let next_elemt = page.pop().unwrap();
            let k = HeritageMonoItemKeyMapper::TxSummary(Some((
                &next_elemt.txid,
                next_elemt.confirmation_time.as_ref(),
            )))
            .key();
            Some(ContinuationToken(k))
        } else {
            None
        };

        Ok(Paginated {
            page,
            continuation_token,
        })
    }

    fn get_balance(&self) -> Result<Option<HeritageWalletBalance>> {
        log::debug!("HeritageMemoryDatabase::get_balance");
        let key = HeritageMonoItemKeyMapper::WalletBalance.key();
        Ok(self.table.read().unwrap().get(&key).map(|b| {
            b.downcast_ref::<HeritageWalletBalance>()
                .expect("this is a HeritageWalletBalance")
                .clone()
        }))
    }

    fn set_balance(&mut self, new_balance: &HeritageWalletBalance) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::set_balance - new_balance={new_balance:?}");
        let key = HeritageMonoItemKeyMapper::WalletBalance.key();
        self.table
            .write()
            .unwrap()
            .insert(key, Box::new(new_balance.clone()));
        Ok(())
    }

    fn get_fee_rate(&self) -> Result<Option<FeeRate>> {
        log::debug!("HeritageMemoryDatabase::get_fee_rate");
        let key = HeritageMonoItemKeyMapper::FeeRate.key();
        Ok(self.table.read().unwrap().get(&key).map(|b| {
            b.downcast_ref::<FeeRate>()
                .expect("this is a FeeRate")
                .clone()
        }))
    }

    fn set_fee_rate(&mut self, new_fee_rate: &FeeRate) -> Result<()> {
        log::debug!("HeritageMemoryDatabase::set_fee_rate - new_fee_rate={new_fee_rate:?}");
        let key = HeritageMonoItemKeyMapper::FeeRate.key();
        self.table
            .write()
            .unwrap()
            .insert(key, Box::new(new_fee_rate.clone()));
        Ok(())
    }

    fn get_block_inclusion_objective(&self) -> Result<Option<BlockInclusionObjective>> {
        log::debug!("HeritageMemoryDatabase::get_block_inclusion_objective");
        let key = HeritageMonoItemKeyMapper::BlockInclusionObjective.key();
        Ok(self.table.read().unwrap().get(&key).map(|b| {
            b.downcast_ref::<BlockInclusionObjective>()
                .expect("this is a FeeRate")
                .clone()
        }))
    }

    fn set_block_inclusion_objective(
        &mut self,
        new_objective: BlockInclusionObjective,
    ) -> Result<()> {
        log::debug!(
            "HeritageMemoryDatabase::set_block_inclusion_objective - new_objective={new_objective:?}"
        );
        let key = HeritageMonoItemKeyMapper::BlockInclusionObjective.key();
        self.table
            .write()
            .unwrap()
            .insert(key, Box::new(new_objective));
        Ok(())
    }
}
