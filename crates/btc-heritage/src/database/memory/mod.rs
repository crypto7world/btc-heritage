use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::RwLock,
};
extern crate bdk as crate_bdk;
use crate_bdk::BlockTime;

use crate::{
    account_xpub::AccountXPubId,
    bitcoin::{OutPoint, Txid},
    heritage_wallet::SubwalletConfigId,
};

use super::{PartitionableDatabase, Result, SubdatabaseId};

mod bdk;
mod heritage;

use bdk::HeritageBdkMemoryDatabaseWrapper;

enum HeritageMonoItemKeyMapper<'a> {
    WalletConfig(Option<SubwalletConfigId>),
    UnusedAccountXPub(Option<AccountXPubId>),
    HeritageUtxo(Option<&'a OutPoint>),
    TxSummary(Option<(&'a Txid, Option<&'a BlockTime>)>),
    WalletBalance,
    FeeRate,
    BlockInclusionObjective,
}

impl HeritageMonoItemKeyMapper<'_> {
    fn pk(&self) -> &str {
        match *self {
            HeritageMonoItemKeyMapper::WalletConfig(_) => "wc",
            HeritageMonoItemKeyMapper::UnusedAccountXPub(_) => "uaxpubs",
            HeritageMonoItemKeyMapper::HeritageUtxo(_) => "hutxo",
            HeritageMonoItemKeyMapper::TxSummary(_) => "txsum",
            HeritageMonoItemKeyMapper::WalletBalance => "balance",
            HeritageMonoItemKeyMapper::FeeRate => "feerate",
            HeritageMonoItemKeyMapper::BlockInclusionObjective => "bio",
        }
    }

    fn sk(&self) -> String {
        match *self {
            HeritageMonoItemKeyMapper::WalletConfig(Some(SubwalletConfigId::Current)) => {
                "Current".to_owned()
            }
            HeritageMonoItemKeyMapper::WalletConfig(Some(SubwalletConfigId::Id(id)))
            | HeritageMonoItemKeyMapper::UnusedAccountXPub(Some(id)) => {
                format!("{:0>10}", id)
            }
            HeritageMonoItemKeyMapper::HeritageUtxo(Some(op)) => op.to_string(),
            HeritageMonoItemKeyMapper::TxSummary(Some((txid, confirmation_time))) => format!(
                "{:0>10}#{}",
                confirmation_time
                    .as_ref()
                    .map(|bt| bt.height)
                    .unwrap_or(u32::MAX),
                txid.to_string()
            ),
            _ => "".to_owned(),
        }
    }

    fn key(&self) -> String {
        let pk = self.pk();
        let sk = self.sk();
        [pk, "#", &sk].concat()
    }
}

#[derive(Debug)]
pub struct HeritageMemoryDatabase {
    table: RwLock<BTreeMap<String, Box<dyn Any + Send + Sync>>>,
    subdatabases: RefCell<HashMap<SubdatabaseId, HeritageBdkMemoryDatabaseWrapper>>,
}

impl HeritageMemoryDatabase {
    pub fn new() -> Self {
        Self {
            table: RwLock::new(BTreeMap::new()),
            subdatabases: RefCell::new(HashMap::new()),
        }
    }
}

impl PartitionableDatabase for HeritageMemoryDatabase {
    type SubDatabase = HeritageBdkMemoryDatabaseWrapper;

    fn get_subdatabase(&self, subdatabase_id: SubdatabaseId) -> Result<Self::SubDatabase> {
        Ok(self
            .subdatabases
            .borrow_mut()
            .entry(subdatabase_id)
            .or_insert(HeritageBdkMemoryDatabaseWrapper::new())
            .clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{HeritageMemoryDatabase, PartitionableDatabase, SubdatabaseId};

    macro_rules! impl_heritage_test {
        ($tn: tt) => {
            #[test]
            fn $tn() {
                crate::database::tests::$tn(HeritageMemoryDatabase::new())
            }
        };
    }

    impl_heritage_test!(get_put_subwallet_config);
    impl_heritage_test!(get_subdatabase);
    impl_heritage_test!(get_set_balance);
    impl_heritage_test!(get_set_fee_rate);
    impl_heritage_test!(get_set_block_inclusion_objective);
    impl_heritage_test!(list_obsolete_subwallet_configs);
    impl_heritage_test!(safe_update_current_subwallet_config);
    impl_heritage_test!(transaction);
    impl_heritage_test!(unused_account_xpub_management);
    impl_heritage_test!(heritage_utxo_management);
    impl_heritage_test!(transaction_summaries_management);

    macro_rules! impl_bdk_test {
        ($tn: tt) => {
            #[test]
            fn $tn() {
                let heritage_db = HeritageMemoryDatabase::new();
                let subdb_index = SubdatabaseId("sub".to_owned());
                crate::database::bdk_tests::$tn(
                    heritage_db.get_subdatabase(subdb_index.clone()).unwrap(),
                )
            }
        };
    }

    impl_bdk_test!(test_script_pubkey);
    impl_bdk_test!(test_batch_script_pubkey);
    impl_bdk_test!(test_iter_script_pubkey);
    impl_bdk_test!(test_del_script_pubkey);
    impl_bdk_test!(test_utxo);
    impl_bdk_test!(test_raw_tx);
    impl_bdk_test!(test_tx);
    impl_bdk_test!(test_list_transaction);
    impl_bdk_test!(test_last_index);
    impl_bdk_test!(test_sync_time);
    impl_bdk_test!(test_iter_raw_txs);
    impl_bdk_test!(test_del_path_from_script_pubkey);
    impl_bdk_test!(test_iter_script_pubkeys);
    impl_bdk_test!(test_del_utxo);
    impl_bdk_test!(test_del_raw_tx);
    impl_bdk_test!(test_del_tx);
    impl_bdk_test!(test_del_last_index);
    impl_bdk_test!(test_check_descriptor_checksum);
}
