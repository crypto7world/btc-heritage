use std::sync::Arc;

use btc_heritage::{
    bdk_types,
    bitcoin::{OutPoint, Script, Txid},
    database::{PartitionableDatabase, SubdatabaseId},
    errors::DatabaseError,
    heritage_wallet::SubwalletConfigId,
    AccountXPubId,
};

mod bdk;
mod heritage;

enum KeyMapper<'a> {
    // HeritageWallet DB related
    SubwalletConfig(Option<SubwalletConfigId>),
    UnusedAccountXPub(Option<AccountXPubId>),
    HeritageUtxo(Option<&'a OutPoint>),
    TxSummary(Option<(&'a Txid, Option<&'a bdk_types::BlockTime>)>),
    WalletBalance,
    FeeRate,
    BlockInclusionObjective,
    // bdk::Wallet DB related
    SyncTime,
    Path((Option<bdk_types::KeychainKind>, Option<u32>)),
    Script(Option<&'a Script>),
    Utxo(Option<&'a OutPoint>),
    RawTx(Option<&'a Txid>),
    Transaction(Option<&'a Txid>),
    LastIndex(bdk_types::KeychainKind),
    DescriptorChecksum(bdk_types::KeychainKind),
}

impl KeyMapper<'_> {
    fn pk(&self) -> &str {
        match *self {
            // HeritageWallet DB related
            KeyMapper::SubwalletConfig(_) => "w",
            KeyMapper::UnusedAccountXPub(_) => "x",
            KeyMapper::HeritageUtxo(_) => "h",
            KeyMapper::TxSummary(_) => "y",
            KeyMapper::WalletBalance => "b",
            KeyMapper::FeeRate => "f",
            KeyMapper::BlockInclusionObjective => "o",
            // bdk::Wallet DB related
            KeyMapper::Path(_) => "p",
            KeyMapper::Script(_) => "s",
            KeyMapper::Utxo(_) => "u",
            KeyMapper::RawTx(_) => "r",
            KeyMapper::Transaction(_) => "t",
            KeyMapper::LastIndex(_) => "i",
            KeyMapper::SyncTime => "l",
            KeyMapper::DescriptorChecksum(_) => "d",
        }
    }

    fn sk(&self) -> String {
        match *self {
            // HeritageWallet DB related
            KeyMapper::SubwalletConfig(Some(SubwalletConfigId::Current)) => "c".to_owned(),
            KeyMapper::SubwalletConfig(Some(SubwalletConfigId::Id(id))) => {
                // We use a prefix to ease the query of the obsolete configs
                // We chose "a" because it is before "c" and therefor is does not upset the database order
                // i.e. the "current" subwallet is still after in lexical order
                format!("a{:0>10}", id)
            }
            KeyMapper::UnusedAccountXPub(Some(id)) => {
                format!("{:0>10}", id)
            }
            KeyMapper::HeritageUtxo(Some(op)) => op.to_string(),
            KeyMapper::TxSummary(Some((txid, confirmation_time))) => format!(
                "{:0>10}#{}",
                confirmation_time
                    .as_ref()
                    .map(|bt| bt.height)
                    .unwrap_or(u32::MAX),
                txid.to_string()
            ),
            // bdk::Wallet DB related
            KeyMapper::Path((Some(kk), Some(idx))) => {
                format!("{}#{idx:0>10}", kk.as_byte() as char)
            }
            KeyMapper::Path((Some(kk), None)) => {
                format!("{}#", kk.as_byte() as char)
            }
            KeyMapper::Script(Some(s)) => s.script_hash().to_string(),
            KeyMapper::Utxo(Some(op)) => op.to_string(),
            KeyMapper::RawTx(Some(txid)) | KeyMapper::Transaction(Some(txid)) => txid.to_string(),
            KeyMapper::LastIndex(kk) | KeyMapper::DescriptorChecksum(kk) => {
                (kk.as_byte() as char).to_string()
            }
            _ => String::new(),
        }
    }

    fn key(&self, prefix: &str) -> String {
        let pk = self.pk();
        let sk = self.sk();
        format!("{prefix}#{pk}#{sk}")
    }
}

use super::Database;
#[derive(Debug)]
pub struct HeritageWalletDatabase {
    db: Database,
    prefix: String,
}

impl HeritageWalletDatabase {
    /// Create a brand new [HeritageWalletDatabase] using a wallet id and the common database
    ///
    /// # Errors
    /// Return an error if there is already a database corresponding to `wallet_id`
    pub async fn create(wallet_id: String, db: &Database) -> Result<Self, super::errors::DbError> {
        if db.table_exists(&wallet_id).await? {
            Err(super::errors::DbError::TableAlreadyExists(
                wallet_id.to_owned(),
            ))
        } else {
            let mut hdb = Self::new(wallet_id, db);
            // To ensure the table is effectively created we write something
            hdb.db.put_item("marker_key", &"marker_data").await?;
            Ok(hdb)
        }
    }

    /// Get an [HeritageWalletDatabase] using a wallet id and the common database
    ///
    /// # Errors
    /// Return an error if there is no database corresponding to `wallet_id`
    ///
    /// # Panics
    /// `wallet_id` cannot have the same value as [DEFAULT_TABLE_NAME](super::DEFAULT_TABLE_NAME)
    /// and the function will panic if it is the case
    pub async fn get(wallet_id: String, db: &Database) -> Result<Self, super::errors::DbError> {
        if db.table_exists(&wallet_id).await? {
            Ok(Self::new(wallet_id, db))
        } else {
            Err(super::errors::DbError::TableDoesNotExists(
                wallet_id.to_owned(),
            ))
        }
    }

    fn new(wallet_id: String, db: &Database) -> Self {
        // We don't want to risk conflict with the default table name
        // We will just panic if wallet_id has the same value
        assert_ne!(
            wallet_id,
            super::DEFAULT_TABLE_NAME,
            "wallet_id cannot be \"{}\"",
            super::DEFAULT_TABLE_NAME
        );
        HeritageWalletDatabase {
            db: Database {
                internal_db: Arc::clone(&db.internal_db),
                table_name: Some(wallet_id),
            },
            prefix: String::new(),
        }
    }
}

impl HeritageWalletDatabase {
    fn key(&self, km: &KeyMapper) -> String {
        km.key(&self.prefix)
    }
}

impl PartitionableDatabase for HeritageWalletDatabase {
    type SubDatabase = Self;

    fn get_subdatabase(
        &self,
        subdatabase_id: SubdatabaseId,
    ) -> Result<Self::SubDatabase, DatabaseError> {
        Ok(HeritageWalletDatabase {
            db: Database {
                internal_db: Arc::clone(&self.db.internal_db),
                table_name: self.db.table_name.clone(),
            },
            prefix: subdatabase_id.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ops::Deref;

    use super::{Database, HeritageWalletDatabase, PartitionableDatabase, SubdatabaseId};
    use btc_heritage::bitcoin::Network;

    struct TestEnv {
        db: Database,
        _tmpdir: tempfile::TempDir,
    }
    impl Deref for TestEnv {
        type Target = Database;

        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }

    // Utilitary function that create a temp database that will be removed at the end
    fn setup_test_env() -> TestEnv {
        let tmpdir = tempfile::tempdir().unwrap();
        let db = tokio::runtime::Builder::new_multi_thread()
            .max_blocking_threads(4)
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                Database::new(tmpdir.path(), Network::Regtest)
                    .await
                    .unwrap()
            });
        TestEnv {
            db,
            _tmpdir: tmpdir,
        }
    }

    macro_rules! impl_heritage_test {
        ($tn: tt) => {
            #[test]
            fn $tn() {
                let te = setup_test_env();
                btc_heritage::database::tests::$tn(HeritageWalletDatabase::new(
                    "wallet".to_owned(),
                    &te,
                ))
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
                let te = setup_test_env();
                let heritage_db = HeritageWalletDatabase::new("wallet".to_owned(), &te);
                let subdb_index = SubdatabaseId::from("sub".to_owned());
                btc_heritage::database::bdk_tests::$tn(
                    heritage_db.get_subdatabase(subdb_index).unwrap(),
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
    impl_bdk_test!(test_batch_raw_tx);
    impl_bdk_test!(test_tx);
    impl_bdk_test!(test_batch_tx);
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
