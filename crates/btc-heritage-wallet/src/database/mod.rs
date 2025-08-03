use std::{fmt::Debug, path::Path, sync::Arc, usize};

use btc_heritage::bitcoin::Network;

pub(crate) mod dbitem;
mod dbschema;
pub(crate) mod errors;
mod heritage_db;
mod utils;

use errors::{DbError, Result};
use heritage_service_api_client::TokenCache;
use redb::{ReadOnlyTable, ReadableTable, Table, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};
use utils::prepare_data_dir;

pub use dbitem::{DatabaseItem, DatabaseSingleItem};
pub use heritage_db::HeritageWalletDatabase;

const DEFAULT_TABLE_NAME: &'static str = "heritage";
const DEFAULT_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new(DEFAULT_TABLE_NAME);
const TOKEN_KEY: &'static str = "api_auth_tokens";

pub enum DatabaseTransactionOperation {
    Update(String, Vec<u8>),
    Delete(String),
    CompareAndSwap {
        key: String,
        old_value: Option<Vec<u8>>,
        new_value: Option<Vec<u8>>,
    },
}
impl Debug for DatabaseTransactionOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Update(key, _) => f.debug_tuple("Update").field(key).finish(),
            Self::Delete(key) => f.debug_tuple("Delete").field(key).finish(),
            Self::CompareAndSwap { key, .. } => f.debug_tuple("CompareAndSwap").field(key).finish(),
        }
    }
}

#[derive(Debug)]
pub struct DatabaseTransaction(Vec<DatabaseTransactionOperation>);
impl DatabaseTransaction {
    pub fn put_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
        let bytes_value = serde_json::to_vec(item).map_err(|e| DbError::serde(key, e))?;
        self.0.push(DatabaseTransactionOperation::CompareAndSwap {
            key: key.to_owned(),
            old_value: None,
            new_value: Some(bytes_value),
        });
        Ok(())
    }

    pub fn update_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
        let bytes_value = serde_json::to_vec(item).map_err(|e| DbError::serde(key, e))?;
        self.0.push(DatabaseTransactionOperation::Update(
            key.to_owned(),
            bytes_value,
        ));
        Ok(())
    }

    pub fn delete_item(&mut self, key: &str) {
        self.0
            .push(DatabaseTransactionOperation::Delete(key.to_owned()));
    }

    pub fn compare_and_swap<T: Serialize + DeserializeOwned>(
        &mut self,
        key: &str,
        old_value: Option<&T>,
        new_value: Option<&T>,
    ) -> Result<()> {
        let old_value = old_value
            .map(|v| serde_json::to_vec(v))
            .transpose()
            .map_err(|e| DbError::serde(key, e))?;
        let new_value = new_value
            .map(|v| serde_json::to_vec(v))
            .transpose()
            .map_err(|e| DbError::serde(key, e))?;

        let key = key.to_owned();
        self.0.push(DatabaseTransactionOperation::CompareAndSwap {
            key,
            old_value,
            new_value,
        });
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Database {
    internal_db: Arc<redb::Database>,
    table_name: Option<Arc<str>>,
}
impl Database {
    pub fn database_path(data_dir: &Path, network: Network) -> std::path::PathBuf {
        let database_name = network.to_string().to_lowercase();
        let mut database_path = data_dir.to_path_buf();
        database_path.push(format!("{database_name}.redb"));
        database_path
    }
    // pub async fn new(data_dir: &Path, network: Network) -> Result<Self> {
    //     tokio::task::spawn_blocking(move || Database::_new(data_dir, network))
    //         .await
    //         .unwrap()
    // }
    pub fn new(data_dir: &Path, network: Network) -> Result<Self> {
        prepare_data_dir(data_dir)?;

        let database_path = Self::database_path(data_dir, network);
        let db = redb::Database::create(database_path.as_path()).map_err(|e| {
            DbError::Generic(format!(
                "Cannot create database at {}: {}",
                database_path.as_path().display(),
                e.to_string()
            ))
        })?;

        log::debug!("Main database opened successfully");

        let mut database = Database {
            internal_db: Arc::new(db),
            table_name: None,
        };

        // Perform automatic schema migration
        dbschema::migrate_database_if_needed(&mut database)?;

        Ok(database)
    }

    pub fn begin_transac(&self) -> DatabaseTransaction {
        DatabaseTransaction(Vec::new())
    }

    pub fn commit_transac(&mut self, transac: DatabaseTransaction) -> Result<()> {
        log::info!("Database::commit_transac - {} ops", transac.0.len());
        let txn = self.internal_db.begin_write()?;
        let tx_res = 'txn: {
            let mut table = txn.open_table(self.table_def())?;
            for (idx, op) in transac.0.into_iter().enumerate() {
                let op_string = format!("{op:?}");
                match &op {
                    DatabaseTransactionOperation::Update(key, value) => {
                        match table.insert(key.as_str(), value.as_slice()) {
                            Ok(_) => (),
                            Err(e) => {
                                log::error!("Operation {op_string} => {e}");
                                break 'txn Err(DbError::TransactionFailed {
                                    idx,
                                    op,
                                    reason: e.to_string(),
                                });
                            }
                        }
                    }
                    DatabaseTransactionOperation::Delete(key) => match table.remove(key.as_str()) {
                        Ok(_) => (),
                        Err(e) => {
                            log::error!("Operation {op_string} => {e}");
                            break 'txn Err(DbError::TransactionFailed {
                                idx,
                                op,
                                reason: e.to_string(),
                            });
                        }
                    },
                    DatabaseTransactionOperation::CompareAndSwap {
                        key,
                        old_value,
                        new_value,
                    } => {
                        match Database::_compare_and_swap_inner(
                            &mut table,
                            &key,
                            old_value.as_deref(),
                            new_value.as_deref(),
                        ) {
                            Ok(_) => (),
                            Err(e) => {
                                log::error!("Operation {op_string} => {e}");
                                break 'txn Err(DbError::TransactionFailed {
                                    idx,
                                    op,
                                    reason: e.to_string(),
                                });
                            }
                        }
                    }
                };
                log::debug!("Operation {op_string} => ok");
            }
            Ok(())
        };
        if tx_res.is_ok() {
            txn.commit()?;
            log::info!("Database::commit_transac - Success");
        } else {
            txn.abort()?;
            log::warn!("Database::commit_transac - Failure");
        };
        tx_res
    }

    pub fn table_exists(&self, table_name: &str) -> Result<bool> {
        let table_def: TableDefinition<'_, &'static str, &'static [u8]> =
            TableDefinition::new(table_name);
        match self.internal_db.begin_read()?.open_table(table_def) {
            Ok(_) => Ok(true),
            Err(e) => match e {
                redb::TableError::TableDoesNotExist(_) => Ok(false),
                _ => Err(e.into()),
            },
        }
    }

    // pub async fn table_exists(&self, table_name: &str) -> Result<bool> {
    //     tokio::task::block_in_place(|| self._table_exists(table_name))
    // }

    pub fn drop_table(&mut self, table_name: &str) -> Result<bool> {
        let txn = self.internal_db.begin_write()?;
        let table_exist = {
            let table_def: TableDefinition<'_, &'static str, &'static [u8]> =
                TableDefinition::new(table_name);
            txn.delete_table(table_def)?
        };
        txn.commit()?;
        Ok(table_exist)
    }

    // pub async fn drop_table(&mut self, table_name: &str) -> Result<bool> {
    //     tokio::task::block_in_place(|| self._drop_table(table_name))
    // }

    pub fn get_item<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        if let Some(table) = self._read_tnx()? {
            Ok(table
                .get(key)?
                .map(|sl| serde_json::from_slice(sl.value()))
                .transpose()
                .map_err(|e| DbError::serde(key, e))?)
        } else {
            Ok(None)
        }
    }

    // pub async fn get_item<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
    //     tokio::task::block_in_place(|| self._get_item(key))
    // }

    pub fn put_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
        let bytes_value = serde_json::to_vec(item).map_err(|e| DbError::serde(key, e))?;
        self._put_item(key, bytes_value)
    }

    fn _put_item(&mut self, key: &str, bytes: Vec<u8>) -> Result<()> {
        let txn = self.internal_db.begin_write()?;
        let put_ok = {
            let mut table = txn.open_table(self.table_def())?;
            match Self::_compare_and_swap_inner(&mut table, key, None, Some(bytes.as_slice())) {
                Ok(_) => true,
                Err(e) => match e {
                    DbError::CompareAndSwapError(_) => false,
                    _ => return Err(e),
                },
            }
        };
        if put_ok {
            txn.commit()?;
            Ok(())
        } else {
            txn.abort()?;
            Err(DbError::KeyAlreadyExists(key.to_owned()))
        }
    }

    // pub async fn put_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
    //     tokio::task::block_in_place(|| self._put_item(key, item))
    // }

    pub fn update_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<bool> {
        let bytes_value = serde_json::to_vec(item).map_err(|e| DbError::serde(key, e))?;
        self._update_item(key, bytes_value)
    }
    fn _update_item(&mut self, key: &str, bytes: Vec<u8>) -> Result<bool> {
        let txn = self.internal_db.begin_write()?;
        let exist = {
            let mut table = txn.open_table(self.table_def())?;
            let exist = table.insert(key, bytes.as_slice())?.is_some();
            exist
        };
        txn.commit()?;
        Ok(exist)
    }

    // pub async fn update_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<bool> {
    //     tokio::task::block_in_place(|| self._update_item(key, item))
    // }

    pub fn delete_item<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        let txn = self.internal_db.begin_write()?;
        let old_value = {
            let mut table = txn.open_table(self.table_def())?;
            let old_value = table
                .remove(key)?
                .map(|sl| serde_json::from_slice(&sl.value()))
                .transpose()
                .map_err(|e| DbError::serde(key, e))?;
            old_value
        };
        txn.commit()?;
        Ok(old_value)
    }

    // pub async fn delete_item<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
    //     tokio::task::block_in_place(|| self._delete_item(key))
    // }

    pub fn compare_and_swap<T: Serialize + DeserializeOwned>(
        &mut self,
        key: &str,
        old_value: Option<&T>,
        new_value: Option<&T>,
    ) -> Result<()> {
        let txn = self.internal_db.begin_write()?;
        {
            let mut table = txn.open_table(self.table_def())?;

            let old_value = old_value
                .map(|v| serde_json::to_vec(v))
                .transpose()
                .map_err(|e| DbError::serde(key, e))?;
            let new_value = new_value
                .map(|v| serde_json::to_vec(v))
                .transpose()
                .map_err(|e| DbError::serde(key, e))?;
            Self::_compare_and_swap_inner(
                &mut table,
                key,
                old_value.as_deref(),
                new_value.as_deref(),
            )?;
        }
        txn.commit()?;
        Ok(())
    }

    // pub async fn compare_and_swap<T: Serialize + DeserializeOwned>(
    //     &mut self,
    //     key: &str,
    //     old_value: Option<&T>,
    //     new_value: Option<&T>,
    // ) -> Result<()> {
    //     tokio::task::block_in_place(|| self._compare_and_swap(key, old_value, new_value))
    // }

    pub fn contains_key(&self, key: &str) -> Result<bool> {
        if let Some(table) = self._read_tnx()? {
            Ok(table.get(key)?.is_some())
        } else {
            Ok(false)
        }
    }

    // pub async fn contains_key(&self, key: &str) -> Result<bool> {
    //     tokio::task::block_in_place(|| self._contains_key(key))
    // }

    /// Returns all the object in the DB whose key begin with `prefix`
    ///
    /// # Errors
    /// Will throw an error if the results from the query are not homogenous (all of the same type).
    /// Will also throw an error if `prefix` is the empty string
    pub fn query<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
        self.query_inner(prefix, None, None, true).map(|(r, _)| r)
    }
    // pub async fn query<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
    //     tokio::task::block_in_place(|| self._query(prefix))
    // }

    /// Returns a page of size `page_size` of the object in the DB whose key begin with `prefix`,
    /// with an optional String that represent the next key if it exist.
    /// A previously returned next key can be used as `start_key` to query the next page
    ///
    /// # Errors
    /// Will throw an error if the results from the query are not homogenous (all of the same type).
    /// Will also throw an error if `prefix` is the empty string
    pub fn query_page<T: DeserializeOwned>(
        &self,
        prefix: &str,
        page_size: usize,
        start_key: Option<String>,
    ) -> Result<(Vec<T>, Option<String>)> {
        self.query_inner(prefix, Some(page_size), start_key, true)
    }
    // pub async fn query_page<T: DeserializeOwned>(
    //     &self,
    //     prefix: &str,
    //     page_size: usize,
    //     start_key: Option<String>,
    // ) -> Result<(Vec<T>, Option<String>)> {
    //     tokio::task::block_in_place(|| self._query_page(prefix, page_size, start_key))
    // }

    /// Like [Self::query] but the DB is tranversed in reverse order
    ///
    /// # Errors
    /// Will throw an error if the results from the query are not homogenous (all of the same type).
    /// Will also throw an error if `prefix` is the empty string
    pub fn query_rev<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
        self.query_inner(prefix, None, None, false).map(|(r, _)| r)
    }
    // pub async fn query_rev<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
    //     tokio::task::block_in_place(|| self._query_rev(prefix))
    // }

    /// Like [Self::query_page] but the DB is tranversed in reverse order
    ///
    /// # Errors
    /// Will throw an error if the results from the query are not homogenous (all of the same type).
    /// Will also throw an error if `prefix` is the empty string
    pub fn query_page_rev<T: DeserializeOwned>(
        &self,
        prefix: &str,
        page_size: usize,
        start_key: Option<String>,
    ) -> Result<(Vec<T>, Option<String>)> {
        self.query_inner(prefix, Some(page_size), start_key, false)
    }
    // pub async fn query_page_rev<T: DeserializeOwned>(
    //     &self,
    //     prefix: &str,
    //     page_size: usize,
    //     start_key: Option<String>,
    // ) -> Result<(Vec<T>, Option<String>)> {
    //     tokio::task::block_in_place(|| self._query_page_rev(prefix, page_size, start_key))
    // }

    fn query_inner<T: DeserializeOwned>(
        &self,
        prefix: &str,
        page_size: Option<usize>,
        start_key: Option<String>,
        scan_forward: bool,
    ) -> Result<(Vec<T>, Option<String>)> {
        if prefix.is_empty() {
            return Err(DbError::EmptyPrefix);
        }
        if let Some(table) = self._read_tnx()? {
            let mut prefix_with_additionnal_max_char = prefix.to_owned();
            prefix_with_additionnal_max_char.push(char::MAX);

            let lower_bound = prefix;
            let upper_bound = prefix_with_additionnal_max_char.as_str();

            let range_bound = if let Some(ref start_key) = start_key {
                if scan_forward {
                    start_key.as_str()..=upper_bound
                } else {
                    lower_bound..=start_key.as_str()
                }
            } else {
                lower_bound..=upper_bound
            };

            let fmap = |e: std::result::Result<
                (redb::AccessGuard<'_, &str>, redb::AccessGuard<'_, &[u8]>),
                redb::StorageError,
            >| {
                e.ok().map(|(key, value)| {
                    Ok((
                        key.value().to_owned(),
                        serde_json::from_slice(&value.value())
                            .map_err(|e| DbError::serde(key.value(), e))?,
                    ))
                })
            };

            let range = table.range(range_bound)?;
            let page: Result<Vec<(String, T)>> = match (page_size, scan_forward) {
                (None, true) => range.filter_map(fmap).collect(),
                (None, false) => range.rev().filter_map(fmap).collect(),
                (Some(page_size), true) => range.take(page_size + 1).filter_map(fmap).collect(),
                (Some(page_size), false) => {
                    range.rev().take(page_size + 1).filter_map(fmap).collect()
                }
            };
            let mut page = page?;

            let next_key = if page_size.is_some_and(|page_size| page.len() > page_size) {
                Some(page.pop().unwrap().0)
            } else {
                None
            };
            Ok((page.into_iter().map(|(_, t)| t).collect(), next_key))
        } else {
            Ok((vec![], None))
        }
    }

    /// List all the keys in the DB
    /// If `prefix` is [Some] and not the empty string, returns only keys that begin with `prefix`
    pub fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        if let Some(table) = self._read_tnx()? {
            if prefix.is_some_and(|s| !s.is_empty()) {
                let prefix = prefix.unwrap();
                let mut prefix_with_next_last_char = prefix.to_owned();
                let last_char =
                    prefix_with_next_last_char.remove(prefix_with_next_last_char.len() - 1);
                let next_last_char = (last_char as u8 + 1) as char;
                prefix_with_next_last_char.push(next_last_char);

                Ok(table
                    .range(prefix..prefix_with_next_last_char.as_str())?
                    .filter_map(|e| {
                        let k = e.ok().map(|(key, _)| key.value().to_owned());
                        if k.as_ref().is_some_and(|s| s.starts_with(prefix)) {
                            k
                        } else {
                            None
                        }
                    })
                    .collect())
            } else {
                Ok(table
                    .iter()?
                    .filter_map(|e| e.ok().map(|(key, _)| key.value().to_owned()))
                    .collect())
            }
        } else {
            Ok(vec![])
        }
    }
    /// Executes a database operation in a blocking context using tokio's blocking thread pool
    ///
    /// This function is used to perform database operations that may block the async runtime
    /// by moving them to a dedicated blocking thread pool. This is essential for maintaining
    /// async performance when dealing with potentially blocking database operations.
    ///
    /// # Parameters
    ///
    /// * `f` - A closure that takes a [Database] and returns a result of type `R`
    ///
    /// # Returns
    ///
    /// Returns the result of the closure execution
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use btc_heritage_wallet::Database;
    /// # async fn example(db: Database) -> Result<(), Box<dyn std::error::Error>> {
    /// let result = db.blocking_operation(|db| {
    ///     // Perform blocking database operation here
    ///     db.get_item::<String>("some_key")
    /// }).await;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the spawned blocking task fails to execute (which should be rare
    /// and indicates a serious system issue)
    pub async fn blocking_operation<
        R: Send + 'static,
        F: FnOnce(Database) -> R + Send + 'static,
    >(
        self,
        f: F,
    ) -> R {
        tokio::task::spawn_blocking(move || f(self)).await.unwrap()
    }

    fn _read_tnx(&self) -> Result<Option<ReadOnlyTable<&'static str, &'static [u8]>>> {
        Ok(
            (match self.internal_db.begin_read()?.open_table(self.table_def()) {
                Ok(table) => Ok(Some(table)),
                Err(e) => match e {
                    redb::TableError::TableDoesNotExist(_) => return Ok(None),
                    _ => Err(e),
                },
            })?,
        )
    }

    fn table_def(&self) -> TableDefinition<&'static str, &'static [u8]> {
        self.table_name
            .as_ref()
            .map(|s| TableDefinition::new(s.as_ref()))
            .unwrap_or(DEFAULT_TABLE)
    }

    fn _compare_and_swap_inner(
        table: &mut Table<&str, &[u8]>,
        key: &str,
        old_value: Option<&[u8]>,
        new_value: Option<&[u8]>,
    ) -> Result<()> {
        if table.get(key)?.as_ref().map(|g| g.value()) == old_value {
            if let Some(v) = new_value {
                table.insert(key, v)?;
            } else {
                table.remove(key)?;
            }
            Ok(())
        } else {
            Err(DbError::CompareAndSwapError(key.to_owned()))
        }
    }
}

impl TokenCache for Database {
    async fn save_tokens(
        &mut self,
        tokens: &heritage_service_api_client::Tokens,
    ) -> core::result::Result<(), heritage_service_api_client::Error> {
        let token_bytes = serde_json::to_vec(tokens)
            .map_err(|e| DbError::serde(TOKEN_KEY, e))
            .map_err(|e| {
                log::error!("{e}");
                heritage_service_api_client::Error::TokenCacheWriteError(e.to_string())
            })?;
        self.clone()
            .blocking_operation(move |mut db| {
                db._update_item(TOKEN_KEY, token_bytes).map_err(|e| {
                    log::error!("{e}");
                    heritage_service_api_client::Error::TokenCacheWriteError(e.to_string())
                })
            })
            .await?;
        Ok(())
    }

    async fn load_tokens(
        &self,
    ) -> core::result::Result<
        Option<heritage_service_api_client::Tokens>,
        heritage_service_api_client::Error,
    > {
        self.clone()
            .blocking_operation(move |db| {
                db.get_item(TOKEN_KEY).map_err(|e| {
                    log::error!("{e}");
                    heritage_service_api_client::Error::TokenCacheReadError(e.to_string())
                })
            })
            .await
    }

    async fn clear(&mut self) -> core::result::Result<bool, heritage_service_api_client::Error> {
        Ok(self
            .clone()
            .blocking_operation(move |mut db| {
                db.delete_item::<heritage_service_api_client::Tokens>(TOKEN_KEY)
                    .map_err(|e| {
                        log::error!("{e}");
                        heritage_service_api_client::Error::TokenCacheWriteError(e.to_string())
                    })
            })
            .await?
            .is_some())
    }
}
