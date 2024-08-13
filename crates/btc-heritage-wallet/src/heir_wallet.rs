use serde::{Deserialize, Serialize};

use crate::{
    database::DatabaseItem,
    errors::{Error, Result},
    heritage_provider::AnyHeritageProvider,
    key_provider::{AnyKeyProvider, KeyProvider},
    BoundFingerprint, Broadcaster, Heritage, HeritageProvider,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct HeirWallet {
    name: String,
    key_provider: AnyKeyProvider,
    heritage_provider: AnyHeritageProvider,
}
impl HeirWallet {
    pub fn new(
        name: String,
        key_provider: AnyKeyProvider,
        heritage_provider: AnyHeritageProvider,
    ) -> Result<Self> {
        if heritage_provider.is_none() && key_provider.is_none() {
            return Err(Error::NoComponent);
        }
        if !heritage_provider.is_none()
            && !key_provider.is_none()
            && heritage_provider.fingerprint()? != key_provider.fingerprint()?
        {
            return Err(Error::IncoherentFingerprints);
        }

        Ok(Self {
            name,
            key_provider,
            heritage_provider,
        })
    }
}

crate::database::dbitem::impl_db_item!(HeirWallet, "heirwallet#", "default_heirwallet_name");
crate::key_provider::impl_key_provider!(HeirWallet);
crate::heritage_provider::impl_heritage_provider!(HeirWallet);

impl BoundFingerprint for HeirWallet {
    fn fingerprint(&self) -> Result<btc_heritage::bitcoin::bip32::Fingerprint> {
        if !self.key_provider.is_none() {
            return self.key_provider.fingerprint();
        }
        if !self.heritage_provider.is_none() {
            return self.heritage_provider.fingerprint();
        }
        unreachable!("Having both part at None is not allowed")
    }
}
