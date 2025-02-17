use serde::{Deserialize, Serialize};

use crate::{
    database::{errors::DbError, DatabaseItem},
    errors::{Error, Result},
    key_provider::{AnyKeyProvider, KeyProvider},
    online_wallet::{AnyOnlineWallet, OnlineWallet},
    BoundFingerprint,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Wallet {
    name: String,
    key_provider: AnyKeyProvider,
    online_wallet: AnyOnlineWallet,
    #[serde(default)]
    fingerprints_controlled: bool,
}

impl Wallet {
    pub fn new(
        name: String,
        key_provider: AnyKeyProvider,
        online_wallet: AnyOnlineWallet,
    ) -> Result<Self> {
        if online_wallet.is_none() && key_provider.is_none() {
            Err(Error::NoComponent)
        } else {
            let mut wallet = Self {
                name,
                key_provider,
                online_wallet,
                fingerprints_controlled: false,
            };
            wallet.control_fingerprints()?;
            Ok(wallet)
        }
    }

    fn control_fingerprints(&mut self) -> Result<()> {
        if !self.fingerprints_controlled {
            if !self.key_provider.is_none() && !self.online_wallet.is_none() {
                let online_fp = (match self.online_wallet.fingerprint() {
                    Ok(fp) => Ok(Some(fp)),
                    Err(e) => match e {
                        Error::OnlineWalletFingerprintNotPresent => Ok(None),
                        _ => Err(e),
                    },
                })?;
                if let Some(online_fp) = online_fp {
                    let offline_fp = self.key_provider.fingerprint()?;
                    if online_fp != offline_fp {
                        // The Fingerprint are different!!!
                        // quit the verification
                        return Err(Error::IncoherentFingerprints);
                    }
                    // Let the function continue to mark fingerprints_controlled = true
                } else {
                    // We cannot control the FPs for now, quit the verification
                    return Ok(());
                }
            }
            // We are here because:
            // - Either key_provider or online_wallet is none
            // - They are both not_none, and online_wallet have a fingerprint, and it is coherent with the key_provider
            self.fingerprints_controlled = true;
        }
        Ok(())
    }
}

crate::database::dbitem::impl_db_item!(
    Wallet,
    "wallet#",
    "default_wallet_name"
    async fn load(db: &crate::Database, name: &str) -> crate::database::errors::Result<Self> {
        let key = Self::name_to_key(name);
        let mut wallet = db
            .get_item::<Self>(&key).await?
            .ok_or(crate::database::errors::DbError::KeyDoesNotExists(key))?;
        wallet.control_fingerprints().map_err(|e|DbError::generic(e))?;
        Ok(wallet)
    }
    async fn delete(&self, db: &mut crate::Database) -> crate::database::errors::Result<()> {
        if let AnyOnlineWallet::Local(lw) = &self.online_wallet{
            lw.delete(db).await?;
        }
        db.delete_item::<Self>(&Self::name_to_key(self.name())).await?;
        Ok(())
    }
);
crate::key_provider::impl_key_provider!(Wallet);
crate::online_wallet::impl_online_wallet!(Wallet);

impl BoundFingerprint for Wallet {
    fn fingerprint(&self) -> Result<btc_heritage::bitcoin::bip32::Fingerprint> {
        if !self.key_provider.is_none() {
            return self.key_provider.fingerprint();
        }
        if !self.online_wallet.is_none() {
            return self.online_wallet.fingerprint();
        }
        unreachable!("Having both part at None is not allowed")
    }
}
