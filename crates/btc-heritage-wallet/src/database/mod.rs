use std::path::Path;

use crate::errors::{Error, Result};
use btc_heritage::bitcoin::Network;

pub(crate) mod dbitem;
mod utils;
use heritage_api_client::TokenCache;
use redb::{ReadableTable, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};
use utils::prepare_data_dir;

pub use dbitem::DatabaseItem;

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("heritage");
const TOKEN_KEY: &'static str = "api_auth_tokens";

#[derive(Debug)]
pub struct Database(redb::Database);

impl Database {
    pub fn new(data_dir: &Path, network: Network) -> Result<Self> {
        prepare_data_dir(data_dir)?;

        // We will maintain different DBs for each network
        let database_name = network.to_string().to_lowercase();
        let mut database_path = data_dir.to_path_buf();
        database_path.push(format!("{database_name}.redb"));

        let db = redb::Database::create(database_path.as_path()).map_err(|e| {
            Error::Generic(format!(
                "Cannot create database at {}: {}",
                database_path.as_path().display(),
                e.to_string()
            ))
        })?;

        log::debug!("Main database opened successfully");

        Ok(Database(db))
    }

    pub fn get_item<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let table = (match self.0.begin_read()?.open_table(TABLE) {
            Ok(table) => Ok(table),
            Err(e) => match e {
                redb::TableError::TableDoesNotExist(_) => return Ok(None),
                _ => Err(e),
            },
        })?;
        Ok(table
            .get(key)?
            .map(|sl| serde_json::from_slice(&sl.value()))
            .transpose()?)
    }

    pub fn put_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
        let bytes_value = serde_json::to_vec(item)?;
        let tx = self.0.begin_write()?;
        let put_ok = {
            let mut table = tx.open_table(TABLE)?;
            if table.get(key)?.is_none() {
                table.insert(key, bytes_value.as_slice())?;
                true
            } else {
                false
            }
        };
        if put_ok {
            tx.commit()?;
            Ok(())
        } else {
            tx.abort()?;
            Err(Error::KeyAlreadyExists(key.to_owned()))
        }
    }

    pub fn update_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<bool> {
        let bytes_value = serde_json::to_vec(item)?;
        let tx = self.0.begin_write()?;
        let exist = {
            let mut table = tx.open_table(TABLE)?;
            let exist = table.insert(key, bytes_value.as_slice())?.is_some();
            exist
        };
        tx.commit()?;
        Ok(exist)
    }

    pub fn delete_item<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        let tx = self.0.begin_write()?;
        let old_value = {
            let mut table = tx.open_table(TABLE)?;
            let old_value = table
                .remove(key)?
                .map(|sl| serde_json::from_slice(&sl.value()))
                .transpose()?;
            old_value
        };
        tx.commit()?;
        Ok(old_value)
    }

    pub fn contains_key(&self, key: &str) -> Result<bool> {
        let table = (match self.0.begin_read()?.open_table(TABLE) {
            Ok(table) => Ok(table),
            Err(e) => match e {
                redb::TableError::TableDoesNotExist(_) => return Ok(false),
                _ => Err(e),
            },
        })?;
        Ok(table.get(key)?.is_some())
    }

    /// Returns all the object in the DB whose key begin with `prefix`
    ///
    /// # Errors
    /// Will throw an error if the results from the query are not homogenous (all of the same type).
    /// Will also throw an error if `prefix` is the empty string
    pub fn query<T: DeserializeOwned>(&self, prefix: &str) -> Result<Vec<T>> {
        if prefix.is_empty() {
            return Err(Error::DatabaseError("prefix must not be empty".to_owned()));
        }
        let table = (match self.0.begin_read()?.open_table(TABLE) {
            Ok(table) => Ok(table),
            Err(e) => match e {
                redb::TableError::TableDoesNotExist(_) => return Ok(vec![]),
                _ => Err(e),
            },
        })?;

        let mut prefix_with_next_last_char = prefix.to_owned();
        let last_char = prefix_with_next_last_char.remove(prefix_with_next_last_char.len() - 1);
        let next_last_char = (last_char as u8 + 1) as char;
        prefix_with_next_last_char.push(next_last_char);

        table
            .range(prefix..prefix_with_next_last_char.as_str())?
            .filter_map(|e| {
                e.ok()
                    .map(|(_, value)| Ok(serde_json::from_slice(&value.value())?))
            })
            .collect::<Result<Vec<_>>>()
    }
    /// List all the keys in the DB
    /// If `prefix` is [Some] and not the empty string, returns only keys that begin with `prefix`
    pub fn list_keys(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let table = (match self.0.begin_read()?.open_table(TABLE) {
            Ok(table) => Ok(table),
            Err(e) => match e {
                redb::TableError::TableDoesNotExist(_) => return Ok(vec![]),
                _ => Err(e),
            },
        })?;

        if prefix.is_some_and(|s| !s.is_empty()) {
            let prefix = prefix.unwrap();
            let mut prefix_with_next_last_char = prefix.to_owned();
            let last_char = prefix_with_next_last_char.remove(prefix_with_next_last_char.len() - 1);
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
    }
}

impl TokenCache for Database {
    fn save_tokens(
        &mut self,
        tokens: &heritage_api_client::Tokens,
    ) -> core::result::Result<(), heritage_api_client::Error> {
        self.update_item(TOKEN_KEY, tokens).map_err(|e| {
            log::error!("{e}");
            heritage_api_client::Error::TokenCacheWriteError(e.to_string())
        })?;
        Ok(())
    }

    fn load_tokens(
        &self,
    ) -> core::result::Result<Option<heritage_api_client::Tokens>, heritage_api_client::Error> {
        self.get_item(TOKEN_KEY).map_err(|e| {
            log::error!("{e}");
            heritage_api_client::Error::TokenCacheReadError(e.to_string())
        })
    }
}
