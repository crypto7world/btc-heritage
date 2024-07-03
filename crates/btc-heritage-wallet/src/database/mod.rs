use crate::errors::{Error, Result};
use btc_heritage::bitcoin::Network;

mod utils;
use serde::{de::DeserializeOwned, Serialize};
use utils::prepare_data_dir;

pub struct Database(sled::Db);

impl Database {
    pub fn new(data_dir: &str, network: Network) -> Result<Self> {
        let mut database_path = prepare_data_dir(&data_dir)?;

        // We will maintain different DBs for each network
        let database_name = network.to_string().to_lowercase();
        database_path.push(format!("{database_name}.sled"));

        let db = sled::open(database_path.as_path()).map_err(|e| {
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
        Ok(self
            .0
            .get(key)?
            .map(|sl| serde_json::from_slice(&sl))
            .transpose()?)
    }

    pub fn put_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<()> {
        let bytes_value = serde_json::to_vec(item)?;
        Ok(self
            .0
            .compare_and_swap(key, None as Option<&[u8]>, Some(bytes_value))?
            .map_err(|cse| {
                log::error!("{cse}");
                Error::KeyAlreadyExists(key.to_owned())
            })?)
    }

    pub fn update_item<T: Serialize>(&mut self, key: &str, item: &T) -> Result<bool> {
        let bytes_value = serde_json::to_vec(item)?;
        let insert_result = self.0.insert(key, bytes_value)?;
        Ok(insert_result.is_some())
    }

    pub fn delete_item<T: DeserializeOwned>(&mut self, key: &str) -> Result<Option<T>> {
        Ok(self
            .0
            .remove(key)?
            .map(|sl| serde_json::from_slice(&sl))
            .transpose()?)
    }

    pub fn contains_key(&self, key: &str) -> Result<bool> {
        Ok(self.0.contains_key(key)?)
    }
}
