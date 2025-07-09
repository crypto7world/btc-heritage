use serde::{Deserialize, Serialize};

use crate::{
    database::DatabaseItem,
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

    pub fn fingerprints_controlled(&self) -> bool {
        self.fingerprints_controlled
    }

    /// Attempts to retry fingerprint control if not already established
    ///
    /// This method will try to synchronize the online wallet's fingerprint if it's not present,
    /// then attempt to control fingerprints by verifying they match between the key provider
    /// and online wallet components.
    ///
    /// # Returns
    ///
    /// Returns `true` if either the online wallet fingerprint was updated during synchronization
    /// or if fingerprints control was successfully established. Returns `false` if fingerprints
    /// were already controlled or if no updates were performed.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Synchronizing the online wallet fingerprint fails
    /// - The fingerprint control process fails
    /// - The fingerprints between components are incoherent
    pub async fn retry_fingerprints_control(&mut self) -> Result<bool> {
        if self.fingerprints_controlled {
            return Ok(false);
        }
        // First attempt to synchronize Online Wallet fingerprints if needed
        let online_wallet_fg_updated = if let Err(Error::OnlineWalletFingerprintNotPresent) =
            self.online_wallet.fingerprint()
        {
            match &mut self.online_wallet {
                AnyOnlineWallet::None => unreachable!(
                    "OnlineWalletNone cannot return Error::OnlineWalletFingerprintNotPresent"
                ),
                AnyOnlineWallet::Service(service_binding) => {
                    service_binding.sync_fingerprint().await?
                }
                AnyOnlineWallet::Local(local_heritage_wallet) => {
                    local_heritage_wallet.sync_fingerprint().await?
                }
            }
        } else {
            false
        };

        self.control_fingerprints()?;

        Ok(online_wallet_fg_updated || self.fingerprints_controlled)
    }
}

crate::database::dbitem::impl_db_item!(
    Wallet,
    "wallet#",
    "default_wallet_name"
    fn delete(&self, db: &mut crate::Database) -> crate::database::errors::Result<()> {
        if let AnyOnlineWallet::Local(lw) = &self.online_wallet{
            lw.delete(db)?;
        }
        db.delete_item::<Self>(&Self::name_to_key(self.name()))?;
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
