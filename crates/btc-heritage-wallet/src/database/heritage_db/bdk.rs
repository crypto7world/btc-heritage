use std::collections::HashMap;

use super::{HeritageWalletDatabase, KeyMapper};
use btc_heritage::{
    bdk_types::{self, TransactionDetails},
    bitcoin::{OutPoint, Script, ScriptBuf, Transaction, Txid},
};

#[derive(Debug)]
pub struct HeritageWalletDatabaseBatch {
    inner: super::super::DatabaseTransaction,
    prefix: String,
}
impl HeritageWalletDatabaseBatch {
    fn key(&self, key_mapper: &KeyMapper) -> String {
        key_mapper.key(&self.prefix)
    }
}

impl bdk_types::BatchOperations for HeritageWalletDatabaseBatch {
    fn set_script_pubkey(
        &mut self,
        script: &Script,
        keychain: bdk_types::KeychainKind,
        child: u32,
    ) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::set_script_pubkey - script={script} keychain={keychain:?} child={child}");
        let key = self.key(&KeyMapper::Script(Some(script)));

        self.inner.update_item(&key, &(keychain, child))?;

        let key = self.key(&KeyMapper::Path((Some(keychain), Some(child))));
        self.inner.update_item(&key, &script.to_bytes())?;
        Ok(())
    }

    fn set_utxo(&mut self, utxo: &bdk_types::LocalUtxo) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::set_utxo - utxo={utxo:?}");
        let key = self.key(&KeyMapper::Utxo(Some(&utxo.outpoint)));
        self.inner.update_item(&key, utxo)?;
        Ok(())
    }

    fn set_raw_tx(&mut self, transaction: &Transaction) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::set_raw_tx - transaction={transaction:?}");
        let key = self.key(&KeyMapper::RawTx(Some(&transaction.txid())));
        self.inner.update_item(&key, transaction)?;
        Ok(())
    }

    fn set_tx(
        &mut self,
        transaction: &bdk_types::TransactionDetails,
    ) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::set_tx - transaction={transaction:?}");
        let key = self.key(&KeyMapper::Transaction(Some(&transaction.txid)));

        // insert the raw_tx if present
        if let Some(ref tx) = transaction.transaction {
            self.set_raw_tx(tx)?;
        }
        // remove the raw tx from the serialized version
        let mut transaction = transaction.clone();
        transaction.transaction = None;
        self.inner.update_item(&key, &transaction)?;
        Ok(())
    }

    fn set_last_index(
        &mut self,
        keychain: bdk_types::KeychainKind,
        value: u32,
    ) -> Result<(), bdk_types::Error> {
        log::debug!(
            "HeritageWalletDatabaseBatch::set_last_index - keychain={keychain:?} value={value}"
        );
        let key = self.key(&KeyMapper::LastIndex(keychain));
        self.inner.update_item(&key, &value)?;
        Ok(())
    }

    fn set_sync_time(&mut self, sync_time: bdk_types::SyncTime) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::set_sync_time - sync_time={sync_time:?}");
        let key = self.key(&KeyMapper::SyncTime);
        self.inner.update_item(&key, &sync_time)?;
        Ok(())
    }

    fn del_script_pubkey_from_path(
        &mut self,
        keychain: bdk_types::KeychainKind,
        child: u32,
    ) -> Result<Option<ScriptBuf>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_script_pubkey_from_path - keychain={keychain:?} child={child}");
        let key = self.key(&KeyMapper::Path((Some(keychain), Some(child))));
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_path_from_script_pubkey(
        &mut self,
        script: &Script,
    ) -> Result<Option<(bdk_types::KeychainKind, u32)>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_path_from_script_pubkey - script={script}");
        let key = self.key(&KeyMapper::Script(Some(script)));
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_utxo(
        &mut self,
        outpoint: &OutPoint,
    ) -> Result<Option<bdk_types::LocalUtxo>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_utxo - outpoint={outpoint:?}");
        let key = self.key(&KeyMapper::Utxo(Some(outpoint)));
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_raw_tx(&mut self, txid: &Txid) -> Result<Option<Transaction>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_raw_tx - txid={txid:?}");
        let key = self.key(&KeyMapper::RawTx(Some(txid)));
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_tx(
        &mut self,
        txid: &Txid,
        include_raw: bool,
    ) -> Result<Option<bdk_types::TransactionDetails>, bdk_types::Error> {
        log::debug!(
            "HeritageWalletDatabaseBatch::del_tx - txid={txid:?} include_raw={include_raw}"
        );
        let key = self.key(&KeyMapper::Transaction(Some(txid)));
        if include_raw {
            self.del_raw_tx(txid)?;
        }
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_last_index(
        &mut self,
        keychain: bdk_types::KeychainKind,
    ) -> Result<Option<u32>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_last_index - keychain={keychain:?}");
        let key = self.key(&KeyMapper::LastIndex(keychain));
        self.inner.delete_item(&key);
        Ok(None)
    }

    fn del_sync_time(&mut self) -> Result<Option<bdk_types::SyncTime>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabaseBatch::del_sync_time");
        let key = self.key(&KeyMapper::SyncTime);
        self.inner.delete_item(&key);
        Ok(None)
    }
}

impl bdk_types::BatchOperations for HeritageWalletDatabase {
    fn set_script_pubkey(
        &mut self,
        script: &Script,
        keychain: bdk_types::KeychainKind,
        child: u32,
    ) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_script_pubkey - script={script} keychain={keychain:?} child={child}");

        let mut transac = self.db.begin_transac();

        let key = self.key(&KeyMapper::Script(Some(script)));
        transac.update_item(&key, &(keychain, child))?;

        let key = self.key(&KeyMapper::Path((Some(keychain), Some(child))));
        transac.update_item(&key, &script.to_bytes())?;

        self.db.commit_transac(transac)?;
        Ok(())
    }

    fn set_utxo(&mut self, utxo: &bdk_types::LocalUtxo) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_utxo - utxo={utxo:?}");
        let key = self.key(&KeyMapper::Utxo(Some(&utxo.outpoint)));
        self.db.update_item(&key, utxo)?;
        Ok(())
    }

    fn set_raw_tx(&mut self, transaction: &Transaction) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_raw_tx - transaction={transaction:?}");
        let key = self.key(&KeyMapper::RawTx(Some(&transaction.txid())));
        self.db.update_item(&key, transaction)?;
        Ok(())
    }

    fn set_tx(
        &mut self,
        transaction: &bdk_types::TransactionDetails,
    ) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_tx - transaction={transaction:?}");
        let key = self.key(&KeyMapper::Transaction(Some(&transaction.txid)));

        // insert the raw_tx if present
        if let Some(ref tx) = transaction.transaction {
            self.set_raw_tx(tx)?;
        }

        // remove the raw tx from the serialized version
        let mut transaction = transaction.clone();
        transaction.transaction = None;
        self.db.update_item(&key, &transaction)?;
        Ok(())
    }

    fn set_last_index(
        &mut self,
        keychain: bdk_types::KeychainKind,
        value: u32,
    ) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_last_index - keychain={keychain:?} value={value}");
        let key = self.key(&KeyMapper::LastIndex(keychain));
        self.db.update_item(&key, &value)?;
        Ok(())
    }

    fn set_sync_time(&mut self, sync_time: bdk_types::SyncTime) -> Result<(), bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::set_sync_time - sync_time={sync_time:?}");
        let key = self.key(&KeyMapper::SyncTime);
        self.db.update_item(&key, &sync_time)?;
        Ok(())
    }

    fn del_script_pubkey_from_path(
        &mut self,
        keychain: bdk_types::KeychainKind,
        child: u32,
    ) -> Result<Option<ScriptBuf>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_script_pubkey_from_path - keychain={keychain:?} child={child}");
        let key = self.key(&KeyMapper::Path((Some(keychain), Some(child))));
        let bytes: Option<Vec<u8>> = self.db.delete_item(&key)?;
        Ok(bytes.map(|b| ScriptBuf::from(b)))
    }

    fn del_path_from_script_pubkey(
        &mut self,
        script: &Script,
    ) -> Result<Option<(bdk_types::KeychainKind, u32)>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_path_from_script_pubkey - script={script}");
        let key = self.key(&KeyMapper::Script(Some(script)));
        Ok(self.db.delete_item(&key)?)
    }

    fn del_utxo(
        &mut self,
        outpoint: &OutPoint,
    ) -> Result<Option<bdk_types::LocalUtxo>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_utxo - outpoint={outpoint:?}");
        let key = self.key(&KeyMapper::Utxo(Some(outpoint)));
        Ok(self.db.delete_item(&key)?)
    }

    fn del_raw_tx(&mut self, txid: &Txid) -> Result<Option<Transaction>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_raw_tx - txid={txid:?}");
        let key = self.key(&KeyMapper::RawTx(Some(txid)));
        Ok(self.db.delete_item(&key)?)
    }

    fn del_tx(
        &mut self,
        txid: &Txid,
        include_raw: bool,
    ) -> Result<Option<bdk_types::TransactionDetails>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_tx - txid={txid:?} include_raw={include_raw}");
        let key = self.key(&KeyMapper::Transaction(Some(txid)));
        let raw_tx = if include_raw {
            self.del_raw_tx(txid)?
        } else {
            None
        };
        Ok(self
            .db
            .delete_item(&key)?
            .map(|mut tx: TransactionDetails| {
                tx.transaction = raw_tx;
                tx
            }))
    }

    fn del_last_index(
        &mut self,
        keychain: bdk_types::KeychainKind,
    ) -> Result<Option<u32>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_last_index - keychain={keychain:?}");
        let key = self.key(&KeyMapper::LastIndex(keychain));
        Ok(self.db.delete_item(&key)?)
    }

    fn del_sync_time(&mut self) -> Result<Option<bdk_types::SyncTime>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::del_sync_time");
        let key = self.key(&KeyMapper::SyncTime);
        Ok(self.db.delete_item(&key)?)
    }
}

impl bdk_types::Database for HeritageWalletDatabase {
    fn check_descriptor_checksum<B: AsRef<[u8]>>(
        &mut self,
        keychain: bdk_types::KeychainKind,
        bytes: B,
    ) -> Result<(), bdk_types::Error> {
        let current_checksum = bytes.as_ref().to_vec();
        let bytes_str = btc_heritage::utils::bytes_to_hex_string(&current_checksum);
        log::debug!(
            "HeritageWalletDatabase::check_descriptor_checksum - keychain={keychain:?} bytes={bytes_str}",
        );
        let key = self.key(&KeyMapper::DescriptorChecksum(keychain));
        let recorded_checksum: Option<Vec<u8>> = self.db.get_item(&key)?;
        if let Some(recorded_checksum) = recorded_checksum {
            if current_checksum != recorded_checksum {
                log::warn!(
                    "ChecksumMismatch: recorded_checksum={} current_checksum={}",
                    btc_heritage::utils::bytes_to_hex_string(recorded_checksum),
                    btc_heritage::utils::bytes_to_hex_string(current_checksum)
                );
                return Err(bdk_types::Error::ChecksumMismatch);
            }
        } else {
            self.db.put_item(&key, &current_checksum)?;
        }
        Ok(())
    }

    fn iter_script_pubkeys(
        &self,
        keychain: Option<bdk_types::KeychainKind>,
    ) -> Result<Vec<ScriptBuf>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::iter_script_pubkeys - keychain={keychain:?}");
        let prefix = self.key(&KeyMapper::Path((keychain, None)));
        let bytes: Vec<Vec<u8>> = self.db.query(&prefix)?;
        Ok(bytes.into_iter().map(|b| ScriptBuf::from(b)).collect())
    }

    fn iter_utxos(&self) -> Result<Vec<bdk_types::LocalUtxo>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::iter_utxos");
        let prefix = self.key(&KeyMapper::Utxo(None));
        Ok(self.db.query(&prefix)?)
    }

    fn iter_raw_txs(&self) -> Result<Vec<Transaction>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::iter_raw_txs");
        let prefix = self.key(&KeyMapper::RawTx(None));
        Ok(self.db.query(&prefix)?)
    }

    fn iter_txs(
        &self,
        include_raw: bool,
    ) -> Result<Vec<bdk_types::TransactionDetails>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::iter_txs - include_raw={include_raw}");
        let prefix = self.key(&KeyMapper::Transaction(None));
        let mut raw_txs: HashMap<Txid, Transaction> = if include_raw {
            self.iter_raw_txs()?
                .into_iter()
                .map(|tx| (tx.txid(), tx))
                .collect()
        } else {
            Default::default()
        };
        let mut result: Vec<TransactionDetails> = self.db.query(&prefix)?;
        if include_raw {
            for tx in result.iter_mut() {
                tx.transaction = raw_txs.remove(&tx.txid)
            }
        }

        Ok(result)
    }

    fn get_script_pubkey_from_path(
        &self,
        keychain: bdk_types::KeychainKind,
        child: u32,
    ) -> Result<Option<ScriptBuf>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_script_pubkey_from_path - keychain={keychain:?} child={child}");
        let key = self.key(&KeyMapper::Path((Some(keychain), Some(child))));
        let bytes: Option<Vec<u8>> = self.db.get_item(&key)?;
        Ok(bytes.map(|b| ScriptBuf::from(b)))
    }

    fn get_path_from_script_pubkey(
        &self,
        script: &Script,
    ) -> Result<Option<(bdk_types::KeychainKind, u32)>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_path_from_script_pubkey - script={script}");
        let key = self.key(&KeyMapper::Script(Some(script)));
        Ok(self.db.get_item(&key)?)
    }

    fn get_utxo(
        &self,
        outpoint: &OutPoint,
    ) -> Result<Option<bdk_types::LocalUtxo>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_utxo - outpoint={outpoint:?}");
        let key = self.key(&KeyMapper::Utxo(Some(outpoint)));
        Ok(self.db.get_item(&key)?)
    }

    fn get_raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_raw_tx - txid={txid:?}");
        let key = self.key(&KeyMapper::RawTx(Some(txid)));
        Ok(self.db.get_item(&key)?)
    }

    fn get_tx(
        &self,
        txid: &Txid,
        include_raw: bool,
    ) -> Result<Option<bdk_types::TransactionDetails>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_tx - txid={txid:?} include_raw={include_raw}");
        let key = self.key(&KeyMapper::Transaction(Some(txid)));
        let raw_tx = if include_raw {
            self.get_raw_tx(txid)?
        } else {
            None
        };
        let tx = self.db.get_item(&key)?;
        Ok(tx.map(|mut tx: bdk_types::TransactionDetails| {
            tx.transaction = raw_tx;
            tx
        }))
    }

    fn get_last_index(
        &self,
        keychain: bdk_types::KeychainKind,
    ) -> Result<Option<u32>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_last_index - keychain={keychain:?}");
        let key = self.key(&KeyMapper::LastIndex(keychain));
        Ok(self.db.get_item(&key)?)
    }

    fn get_sync_time(&self) -> Result<Option<bdk_types::SyncTime>, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::get_sync_time");
        let key = self.key(&KeyMapper::SyncTime);
        Ok(self.db.get_item(&key)?)
    }

    fn increment_last_index(
        &mut self,
        keychain: bdk_types::KeychainKind,
    ) -> Result<u32, bdk_types::Error> {
        log::debug!("HeritageWalletDatabase::increment_last_index - keychain={keychain:?}");
        let key = self.key(&KeyMapper::LastIndex(keychain));
        let txn = self
            .db
            .internal_db
            .begin_write()
            .map_err(crate::database::errors::DbError::from)
            .map_err(|e| {
                log::error!("{e:?}");
                bdk_types::Error::Generic(e.to_string())
            })?;
        let idx = {
            let mut table = txn
                .open_table(self.db.table_def())
                .map_err(crate::database::errors::DbError::from)?;
            let new_value = redb::ReadableTable::get(&table, key.as_str())
                .map_err(crate::database::errors::DbError::from)?
                .map(|sl| serde_json::from_slice::<u32>(&sl.value()))
                .transpose()
                .map_err(|e| crate::database::errors::DbError::serde(key.clone(), e))?
                .map(|idx| idx + 1)
                .unwrap_or(0);

            let bytes_value = serde_json::to_vec(&new_value)
                .map_err(|e| crate::database::errors::DbError::serde(key.clone(), e))?;

            table
                .insert(key.as_str(), bytes_value.as_slice())
                .map_err(crate::database::errors::DbError::from)?;

            new_value
        };
        txn.commit()
            .map_err(crate::database::errors::DbError::from)?;
        Ok(idx)
    }
}

impl bdk_types::BatchDatabase for HeritageWalletDatabase {
    type Batch = HeritageWalletDatabaseBatch;

    fn begin_batch(&self) -> Self::Batch {
        Self::Batch {
            inner: self.db.begin_transac(),
            prefix: self.prefix.clone(),
        }
    }

    fn commit_batch(&mut self, batch: Self::Batch) -> Result<(), bdk_types::Error> {
        self.db.commit_transac(batch.inner)?;
        Ok(())
    }
}
