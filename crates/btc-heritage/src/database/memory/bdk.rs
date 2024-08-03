use core::cell::RefCell;
use std::rc::Rc;

use bdk::{
    bitcoin::{Script, ScriptBuf, Transaction, Txid},
    database::{BatchDatabase, BatchOperations, Database, MemoryDatabase, SyncTime},
    Error, KeychainKind, LocalUtxo, TransactionDetails,
};

#[derive(Debug, Clone)]
pub struct HeritageBdkMemoryDatabaseWrapper(Rc<RefCell<MemoryDatabase>>);
impl HeritageBdkMemoryDatabaseWrapper {
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(MemoryDatabase::new())))
    }
}

impl BatchOperations for HeritageBdkMemoryDatabaseWrapper {
    fn set_script_pubkey(
        &mut self,
        script: &Script,
        keychain: KeychainKind,
        child: u32,
    ) -> Result<(), Error> {
        self.0
            .borrow_mut()
            .set_script_pubkey(script, keychain, child)
    }

    fn set_utxo(&mut self, utxo: &LocalUtxo) -> Result<(), Error> {
        self.0.borrow_mut().set_utxo(utxo)
    }

    fn set_raw_tx(&mut self, transaction: &Transaction) -> Result<(), Error> {
        self.0.borrow_mut().set_raw_tx(transaction)
    }

    fn set_tx(&mut self, transaction: &TransactionDetails) -> Result<(), Error> {
        self.0.borrow_mut().set_tx(transaction)
    }

    fn set_last_index(&mut self, keychain: KeychainKind, value: u32) -> Result<(), Error> {
        self.0.borrow_mut().set_last_index(keychain, value)
    }

    fn set_sync_time(&mut self, sync_time: SyncTime) -> Result<(), Error> {
        self.0.borrow_mut().set_sync_time(sync_time)
    }

    fn del_script_pubkey_from_path(
        &mut self,
        keychain: KeychainKind,
        child: u32,
    ) -> Result<Option<ScriptBuf>, Error> {
        self.0
            .borrow_mut()
            .del_script_pubkey_from_path(keychain, child)
    }

    fn del_path_from_script_pubkey(
        &mut self,
        script: &Script,
    ) -> Result<Option<(KeychainKind, u32)>, Error> {
        self.0.borrow_mut().del_path_from_script_pubkey(script)
    }

    fn del_utxo(&mut self, outpoint: &bdk::bitcoin::OutPoint) -> Result<Option<LocalUtxo>, Error> {
        self.0.borrow_mut().del_utxo(outpoint)
    }

    fn del_raw_tx(&mut self, txid: &Txid) -> Result<Option<Transaction>, Error> {
        self.0.borrow_mut().del_raw_tx(txid)
    }

    fn del_tx(
        &mut self,
        txid: &Txid,
        include_raw: bool,
    ) -> Result<Option<TransactionDetails>, Error> {
        self.0.borrow_mut().del_tx(txid, include_raw)
    }

    fn del_last_index(&mut self, keychain: KeychainKind) -> Result<Option<u32>, Error> {
        self.0.borrow_mut().del_last_index(keychain)
    }

    fn del_sync_time(&mut self) -> Result<Option<SyncTime>, Error> {
        self.0.borrow_mut().del_sync_time()
    }
}

impl Database for HeritageBdkMemoryDatabaseWrapper {
    fn check_descriptor_checksum<B: AsRef<[u8]>>(
        &mut self,
        keychain: KeychainKind,
        bytes: B,
    ) -> Result<(), Error> {
        self.0
            .borrow_mut()
            .check_descriptor_checksum(keychain, bytes)
    }

    fn iter_script_pubkeys(&self, keychain: Option<KeychainKind>) -> Result<Vec<ScriptBuf>, Error> {
        self.0.borrow().iter_script_pubkeys(keychain)
    }

    fn iter_utxos(&self) -> Result<Vec<LocalUtxo>, Error> {
        self.0.borrow().iter_utxos()
    }

    fn iter_raw_txs(&self) -> Result<Vec<Transaction>, Error> {
        self.0.borrow().iter_raw_txs()
    }

    fn iter_txs(&self, include_raw: bool) -> Result<Vec<TransactionDetails>, Error> {
        self.0.borrow().iter_txs(include_raw)
    }

    fn get_script_pubkey_from_path(
        &self,
        keychain: KeychainKind,
        child: u32,
    ) -> Result<Option<ScriptBuf>, Error> {
        self.0.borrow().get_script_pubkey_from_path(keychain, child)
    }

    fn get_path_from_script_pubkey(
        &self,
        script: &Script,
    ) -> Result<Option<(KeychainKind, u32)>, Error> {
        self.0.borrow().get_path_from_script_pubkey(script)
    }

    fn get_utxo(&self, outpoint: &bdk::bitcoin::OutPoint) -> Result<Option<LocalUtxo>, Error> {
        self.0.borrow().get_utxo(outpoint)
    }

    fn get_raw_tx(&self, txid: &Txid) -> Result<Option<Transaction>, Error> {
        self.0.borrow().get_raw_tx(txid)
    }

    fn get_tx(&self, txid: &Txid, include_raw: bool) -> Result<Option<TransactionDetails>, Error> {
        self.0.borrow().get_tx(txid, include_raw)
    }

    fn get_last_index(&self, keychain: KeychainKind) -> Result<Option<u32>, Error> {
        self.0.borrow().get_last_index(keychain)
    }

    fn get_sync_time(&self) -> Result<Option<SyncTime>, Error> {
        self.0.borrow().get_sync_time()
    }

    fn increment_last_index(&mut self, keychain: KeychainKind) -> Result<u32, Error> {
        self.0.borrow_mut().increment_last_index(keychain)
    }
}

impl BatchDatabase for HeritageBdkMemoryDatabaseWrapper {
    type Batch = MemoryDatabase;

    fn begin_batch(&self) -> Self::Batch {
        self.0.borrow().begin_batch()
    }

    fn commit_batch(&mut self, batch: Self::Batch) -> Result<(), Error> {
        self.0.borrow_mut().commit_batch(batch)
    }
}
