use std::ops::{Deref, DerefMut};

use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Address},
    AccountXPub, HeirConfig, PartiallySignedTransaction,
};
use heritage_api_client::{TransactionSummary, Txid};
use serde::{Deserialize, Serialize};

use crate::{
    database::DatabaseItem,
    errors::{Error, Result},
    heritage_provider::AnyHeritageProvider,
    key_provider::{AnyKeyProvider, HeirConfigType, KeyProvider, MnemonicBackup},
    BoundFingerprint, Broadcaster, Heritage, HeritageProvider,
};

macro_rules! impl_heir_like {
    ($name:ident, $key_pref:literal, $default_name_key:literal ) => {
        #[derive(Debug, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(InnerHeir);
        impl $name {
            const DB_KEY_PREFIX: &'static str = $key_pref;
            const DEFAULT_NAME_KEY: &'static str = $default_name_key;
        }
        impl Deref for $name {
            type Target = InnerHeir;

            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }
        impl DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
        impl DatabaseItem for $name {
            fn item_key_prefix() -> &'static str {
                Self::DB_KEY_PREFIX
            }

            fn item_default_name_key_prefix() -> &'static str {
                Self::DEFAULT_NAME_KEY
            }

            fn name(&self) -> &str {
                &self.0.name
            }

            fn rename(&mut self, new_name: String) {
                self.0.name = new_name;
            }
        }
    };
}
impl_heir_like!(Heir, "heir#", "default_heir_name");
impl_heir_like!(HeirWallet, "heir#", "default_heir_name");

impl Heir {
    pub fn new(
        name: String,
        heir_config: HeirConfig,
        key_provider: AnyKeyProvider,
    ) -> Result<Self> {
        Ok(Self(InnerHeir::new(
            name,
            Some(heir_config),
            key_provider,
            AnyHeritageProvider::None,
        )?))
    }
    pub fn heir_config(&self) -> &HeirConfig {
        &self
            .0
            .heir_config
            .as_ref()
            .expect("present for Heir(InnerHeir)")
    }
}
impl HeirWallet {
    pub fn new(
        name: String,
        key_provider: AnyKeyProvider,
        heritage_provider: AnyHeritageProvider,
    ) -> Result<Self> {
        Ok(Self(InnerHeir::new(
            name,
            None,
            key_provider,
            heritage_provider,
        )?))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InnerHeir {
    name: String,
    heir_config: Option<HeirConfig>,
    key_provider: AnyKeyProvider,
    heritage_provider: AnyHeritageProvider,
}

impl InnerHeir {
    fn new(
        name: String,
        heir_config: Option<HeirConfig>,
        key_provider: AnyKeyProvider,
        heritage_provider: AnyHeritageProvider,
    ) -> Result<Self> {
        if !key_provider.is_none()
            && heir_config.is_some()
            && key_provider.fingerprint()? != heir_config.as_ref().unwrap().fingerprint()
        {
            return Err(Error::IncoherentFingerprints);
        }
        if !heritage_provider.is_none()
            && heir_config.is_some()
            && heritage_provider.fingerprint()? != heir_config.as_ref().unwrap().fingerprint()
        {
            return Err(Error::IncoherentFingerprints);
        }
        if !heritage_provider.is_none()
            && !key_provider.is_none()
            && heritage_provider.fingerprint()? != key_provider.fingerprint()?
        {
            return Err(Error::IncoherentFingerprints);
        }
        Ok(Self {
            name,
            heir_config,
            key_provider,
            heritage_provider,
        })
    }
    pub fn key_provider(&self) -> &AnyKeyProvider {
        &self.key_provider
    }
    pub fn heritage_provider(&self) -> &AnyHeritageProvider {
        &self.heritage_provider
    }
    pub fn key_provider_mut(&mut self) -> &mut AnyKeyProvider {
        &mut self.key_provider
    }
    pub fn heritage_provider_mut(&mut self) -> &mut AnyHeritageProvider {
        &mut self.heritage_provider
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
impl KeyProvider for InnerHeir {
    impl_key_provider_fn!(sign_psbt(&self, psbt: &mut PartiallySignedTransaction) -> Result<usize>);
    impl_key_provider_fn!(derive_accounts_xpubs(&self, range: core::ops::Range<u32>) -> Result<Vec<AccountXPub>>);
    impl_key_provider_fn!(derive_heir_config(&self, heir_config_type: HeirConfigType) -> Result<HeirConfig>);
    impl_key_provider_fn!(backup_mnemonic(&self) -> Result<MnemonicBackup>);
}
macro_rules! impl_heritage_provider_fn {
    ($fn_name:ident(&mut $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(&mut $self $(,$a : $t)*) -> $ret {
            $self.heritage_provider.$fn_name($($a),*)
        }
    };
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.heritage_provider.$fn_name($($a),*)
        }
    };

}
impl HeritageProvider for InnerHeir {
    impl_heritage_provider_fn!(list_heritages(&self) -> Result<Vec<Heritage>>);
    impl_heritage_provider_fn!(create_psbt(&self, heritage_id: &str, drain_to: Address) -> Result<(PartiallySignedTransaction, TransactionSummary)>);
}
impl Broadcaster for InnerHeir {
    impl_heritage_provider_fn!(broadcast(&self, psbt: PartiallySignedTransaction) -> Result<Txid>);
}
impl BoundFingerprint for InnerHeir {
    impl_heritage_provider_fn!(fingerprint(&self) -> Result<Fingerprint>);
}
