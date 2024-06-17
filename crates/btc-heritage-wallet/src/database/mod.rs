use crate::errors::{Error, Result};
use btc_heritage::bitcoin::Network;

mod utils;
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
}
