use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Txid},
    heritage_wallet::{TransactionSummary, WalletAddress},
    AccountXPub, HeirConfig, HeritageConfig, HeritageWalletBackup, PartiallySignedTransaction,
};
use serde::{Deserialize, Serialize};

use crate::{
    database::DatabaseItem,
    errors::{Error, Result},
    key_provider::{AnyKeyProvider, HeirConfigType, KeyProvider, MnemonicBackup},
    wallet_online::{AnyWalletOnline, WalletOnline},
    BoundFingerprint, Broadcaster,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Wallet {
    name: String,
    key_provider: AnyKeyProvider,
    online_wallet: AnyWalletOnline,
    fingerprints_controlled: bool,
}

impl Wallet {
    const DB_KEY_PREFIX: &'static str = "wallet#";
    const DEFAULT_NAME_KEY: &'static str = "default_wallet_name";

    pub fn new(
        name: String,
        key_provider: AnyKeyProvider,
        online_wallet: AnyWalletOnline,
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

    pub fn key_provider(&self) -> &AnyKeyProvider {
        &self.key_provider
    }
    pub fn online_wallet(&self) -> &AnyWalletOnline {
        &self.online_wallet
    }
    pub fn key_provider_mut(&mut self) -> &mut AnyKeyProvider {
        &mut self.key_provider
    }
    pub fn online_wallet_mut(&mut self) -> &mut AnyWalletOnline {
        &mut self.online_wallet
    }
}

impl DatabaseItem for Wallet {
    fn item_key_prefix() -> &'static str {
        Self::DB_KEY_PREFIX
    }
    fn item_default_name_key_prefix() -> &'static str {
        Self::DEFAULT_NAME_KEY
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }

    fn rename(&mut self, new_name: String) {
        self.name = new_name;
    }

    fn name_to_key(name: &str) -> String {
        std::format!("{}{name}", Self::item_key_prefix())
    }

    fn load(db: &crate::Database, name: &str) -> Result<Self> {
        let mut wallet = db
            .get_item::<Self>(&Self::name_to_key(name))?
            .ok_or(Error::InexistantItem(name.to_owned()))?;
        wallet.control_fingerprints()?;
        Ok(wallet)
    }
}

macro_rules! impl_key_provider_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            $self.key_provider.$fn_name($($a),*)
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.key_provider.$fn_name($($a),*)
        }
    };
}
impl KeyProvider for Wallet {
    impl_key_provider_fn!(sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>);
    impl_key_provider_fn!(derive_accounts_xpubs(&self, range: core::ops::Range<u32>) -> Result<Vec<AccountXPub>>);
    impl_key_provider_fn!(derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>);
    impl_key_provider_fn!(backup_mnemonic(&self) -> Result<MnemonicBackup>);
}
macro_rules! impl_wallet_online_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            $self.online_wallet.$fn_name($($a),*)
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.online_wallet.$fn_name($($a),*)
        }
    };
}
impl WalletOnline for Wallet {
    impl_wallet_online_fn!(backup_descriptors(&self) -> Result<HeritageWalletBackup>);
    impl_wallet_online_fn!(get_address(&self) -> Result<String>);
    impl_wallet_online_fn!(list_addresses(&self) -> Result<Vec<WalletAddress>>);
    impl_wallet_online_fn!(list_account_xpubs(&self) -> Result<Vec<heritage_api_client::AccountXPubWithStatus>>);
    impl_wallet_online_fn!(feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()>);
    impl_wallet_online_fn!(list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>);
    impl_wallet_online_fn!(set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<HeritageConfig>);
    impl_wallet_online_fn!(sync(&mut self) -> Result<()>);
    impl_wallet_online_fn!(get_wallet_info(&self) -> Result<crate::wallet_online::WalletInfo>);
    impl_wallet_online_fn!(create_psbt(&self, new_tx: heritage_api_client::NewTx) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}
impl Broadcaster for Wallet {
    impl_wallet_online_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
impl BoundFingerprint for Wallet {
    fn fingerprint(&self) -> Result<Fingerprint> {
        if !self.key_provider.is_none() {
            return self.key_provider.fingerprint();
        }
        if !self.online_wallet.is_none() {
            return self.online_wallet.fingerprint();
        }
        unreachable!("Having both part at None is not allowed")
    }
}
