use bip39::Mnemonic;
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Network, Txid},
    heritage_wallet::{DescriptorsBackup, TransactionSummary, WalletAddress},
    AccountXPub, HeirConfig, HeritageConfig, PartiallySignedTransaction,
};
use serde::{Deserialize, Serialize};

use crate::{
    database::Database,
    errors::{Error, Result},
    wallet_offline::{AnyWalletOffline, HeirConfigType, WalletOffline},
    wallet_online::{AnyWalletOnline, WalletOnline},
};

pub trait WalletCommons {
    /// Return the [Fingerprint] of the underlying wallets
    fn fingerprint(&self) -> Result<Option<Fingerprint>>;
    /// Return the intended [Network] of the underlying wallets
    fn network(&self) -> Result<Network>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Wallet {
    name: String,
    offline_wallet: AnyWalletOffline,
    online_wallet: AnyWalletOnline,
}

impl Wallet {
    const DB_KEY_PREFIX: &'static str = "wallet#";

    pub fn new(
        name: String,
        offline_wallet: AnyWalletOffline,
        online_wallet: AnyWalletOnline,
    ) -> Result<Self> {
        if online_wallet.is_none() && offline_wallet.is_none() {
            Err(Error::NoComponent)
        } else if !offline_wallet.is_none() && !online_wallet.is_none() && {
            let online_fp = online_wallet.fingerprint()?;
            let offline_fp = offline_wallet.fingerprint()?;
            online_fp.is_some_and(|online_fp| {
                offline_fp.is_some_and(|offline_fp| online_fp != offline_fp)
            })
        } {
            Err(Error::IncoherentFingerprints)
        } else {
            Ok(Self {
                name,
                offline_wallet,
                online_wallet,
            })
        }
    }

    fn name_to_key(name: &str) -> String {
        format!("{}{name}", Self::DB_KEY_PREFIX)
    }
    pub fn db_list_names(db: &Database) -> Result<Vec<String>> {
        let keys_with_prefix = db.list_keys(Some(Self::DB_KEY_PREFIX))?;
        Ok(keys_with_prefix
            .into_iter()
            .map(|k| {
                k.strip_prefix(Self::DB_KEY_PREFIX)
                    .expect("we asked for keys with this prefix")
                    .to_owned()
            })
            .collect())
    }
    /// Verify that the given Wallet name is not already in the database
    pub fn verify_name_is_free(db: &Database, name: &str) -> Result<()> {
        if db.contains_key(&Self::name_to_key(name))? {
            Err(Error::WalletAlreadyExist(name.to_owned()))
        } else {
            Ok(())
        }
    }

    pub fn create(&self, db: &mut Database) -> Result<()> {
        db.put_item(&Self::name_to_key(&self.name), self)?;
        Ok(())
    }

    pub fn delete(&self, db: &mut Database) -> Result<()> {
        let wallet = db.delete_item::<Wallet>(&Self::name_to_key(&self.name))?;
        log::debug!("{wallet:?}");
        Ok(())
    }

    pub fn save(&self, db: &mut Database) -> Result<()> {
        db.update_item(&Self::name_to_key(&self.name), self)?;
        Ok(())
    }

    pub fn load(db: &Database, name: &str) -> Result<Self> {
        db.get_item(&Self::name_to_key(name))?
            .ok_or(Error::InexistantWallet(name.to_owned()))
    }

    pub fn offline_wallet(&self) -> &AnyWalletOffline {
        &self.offline_wallet
    }
    pub fn online_wallet(&self) -> &AnyWalletOnline {
        &self.online_wallet
    }
    pub fn offline_wallet_mut(&mut self) -> &mut AnyWalletOffline {
        &mut self.offline_wallet
    }
    pub fn online_wallet_mut(&mut self) -> &mut AnyWalletOnline {
        &mut self.online_wallet
    }
}

impl WalletCommons for Wallet {
    fn fingerprint(&self) -> Result<Option<Fingerprint>> {
        if !self.offline_wallet.is_none() {
            return self.offline_wallet.fingerprint();
        }
        if !self.online_wallet.is_none() {
            return self.online_wallet.fingerprint();
        }
        unreachable!("Having both part at None is not allowed")
    }

    fn network(&self) -> Result<Network> {
        todo!()
    }
}

macro_rules! impl_wallet_offline_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            $self.offline_wallet.$fn_name($($a),*)
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.offline_wallet.$fn_name($($a),*)
        }
    };
}
impl WalletOffline for Wallet {
    impl_wallet_offline_fn!(sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>);
    impl_wallet_offline_fn!(derive_accounts_xpubs(&self, range: core::ops::Range<u32>) -> Result<Vec<AccountXPub>>);
    impl_wallet_offline_fn!(derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>);
    impl_wallet_offline_fn!(get_mnemonic(&self) -> Result<Mnemonic>);
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
    impl_wallet_online_fn!(backup_descriptors(&self) -> Result<Vec<DescriptorsBackup>>);
    impl_wallet_online_fn!(get_address(&self) -> Result<String>);
    impl_wallet_online_fn!(list_addresses(&self) -> Result<Vec<WalletAddress>>);
    impl_wallet_online_fn!(list_account_xpubs(&self) -> Result<Vec<heritage_api_client::AccountXPubWithStatus>>);
    impl_wallet_online_fn!(feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> Result<()>);
    impl_wallet_online_fn!(list_heritage_configs(&self) -> Result<Vec<HeritageConfig>>);
    impl_wallet_online_fn!(set_heritage_config(&mut self, new_hc: HeritageConfig) -> Result<()>);
    impl_wallet_online_fn!(sync(&mut self) -> Result<()>);
    impl_wallet_online_fn!(get_wallet_info(&self) -> Result<crate::wallet_online::WalletInfo>);
    impl_wallet_online_fn!(create_psbt(&self, new_tx: heritage_api_client::NewTx) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
    impl_wallet_online_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
