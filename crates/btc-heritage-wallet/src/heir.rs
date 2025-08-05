use btc_heritage::{bitcoin::bip32::Fingerprint, HeirConfig};
use serde::{Deserialize, Serialize};

use crate::{
    database::DatabaseItem,
    errors::Result,
    key_provider::{AnyKeyProvider, KeyProvider},
    BoundFingerprint,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Heir {
    pub name: String,
    pub heir_config: HeirConfig,
    key_provider: AnyKeyProvider,
}

impl Heir {
    pub fn new(name: String, heir_config: HeirConfig, key_provider: AnyKeyProvider) -> Self {
        Self {
            name,
            heir_config,
            key_provider,
        }
    }
    pub fn strip_key_provider(&mut self) {
        self.key_provider = AnyKeyProvider::None;
    }
}
crate::database::dbitem::impl_db_item!(Heir, "heir#", "default_heir_name");

crate::key_provider::impl_key_provider!(Heir);

impl BoundFingerprint for Heir {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.heir_config.fingerprint())
    }
}
