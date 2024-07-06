use crate::errors::{Error, Result};
use btc_heritage::bitcoin::Network;

mod utils;
use redb::{ReadableTable, TableDefinition};
use serde::{de::DeserializeOwned, Serialize};
use utils::prepare_data_dir;

const TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("heritage");
pub struct Database(redb::Database);

impl Database {
    pub fn new(data_dir: &str, network: Network) -> Result<Self> {
        let mut database_path = prepare_data_dir(&data_dir)?;

        // We will maintain different DBs for each network
        let database_name = network.to_string().to_lowercase();
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
}
