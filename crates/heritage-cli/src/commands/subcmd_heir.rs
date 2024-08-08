use core::{any::Any, cell::RefCell};

use btc_heritage_wallet::{
    btc_heritage::{AccountXPub, HeirConfig, SingleHeirPubkey},
    errors::{Error, Result},
    heritage_api_client::{HeritageServiceClient, Tokens},
    AnyKeyProvider, BoundFingerprint, Database, DatabaseItem, Heir, KeyProvider, Language,
    LocalKey, Mnemonic,
};
use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::utils::{ask_user_confirmation, prompt_user_for_password};

/// Sub-command for heirs.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum HeirSubcmd {
    /// Creates a new heir with the chosen heritage-provider and offline key-provider
    Create {
        /// The kind of Heir Configuration to generate or import
        #[arg(long, value_enum, default_value_t=HeirConfigType::Xpub)]
        kind: HeirConfigType,
        /// The Heir Configuration to import (optional if key-provider is not none)
        #[arg(short = 'c', long)]
        heir_config: Option<String>,
        /// Specify the kind of key-provider the wallet will use to manages secrets keys and sign transactions
        #[arg(
            short = 'k',long, value_name = "TYPE", aliases = ["kp"], value_enum, default_value_t=KeyProviderType::Local,
            requires_ifs=[("local", "localgen"), ("none", "heir_config")]
        )]
        key_provider: KeyProviderType,
        /// The mnemonic phrase to restore as a seed for the local key-provider (12, 18 or 24 words).
        #[arg(long, value_name = "WORD", num_args=2..24, group="localgen", required_unless_present_any=["key_provider", "word_count"])]
        seed: Option<Vec<String>>,
        /// The length of the mnemonic phrase to generate as a seed for the local key-provider.
        #[arg(
            long, value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()),
            group="localgen", required_unless_present_any=["key_provider", "seed"]
        )]
        word_count: Option<usize>,
        /// Signal that the seed of the local key-provider should be password-protected.
        #[arg(long, default_value_t = false)]
        with_password: bool,
    },
    PushToService {},
    /// Rename the heir in the database to a new name
    Rename {
        new_name: String,
    },
    /// Remove the heir from the database
    /// {n}/!\ BE AWARE THAT YOU WILL NOT BE ABLE TO RETRIEVE THE SEED IF IT IS NOT BACKED-UP /!\
    Remove {
        #[arg(long)]
        /// Confirm that you know what you are doing and skips verification prompts
        i_understand_what_i_am_doing: bool,
    },
    /// Display the fingerprint of the heir
    Fingerprint,
    /// Display the mnemonic of the heir for backup purpose
    BackupMnemonic,
    /// Display the Heir Configuration for this heir
    HeirConfig,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum HeirConfigType {
    /// Produce an Heir Config consisting of a single PublicKey (deprecated)
    SinglePub,
    /// Produce an Heir Config consisting of an Extended PublicKey
    Xpub,
}
impl From<HeirConfigType> for btc_heritage_wallet::HeirConfigType {
    fn from(value: HeirConfigType) -> Self {
        match value {
            HeirConfigType::SinglePub => btc_heritage_wallet::HeirConfigType::SingleHeirPubkey,
            HeirConfigType::Xpub => btc_heritage_wallet::HeirConfigType::HeirXPubkey,
        }
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum KeyProviderType {
    /// No key provider, the resulting wallet will not be able to sign transactions (it will be watch-only)
    None,
    /// Store the seed in the local database (discouraged unless you know what you do. Please use a password to protect the seed)
    Local,
    // Ledger cannot be supported yet
    // /// Use a Ledger hardware-wallet device
    // Ledger,
}

impl super::CommandExecutor for HeirSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (mut db, heir_name, gargs, service_gargs): (
            Database,
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
        ) = *params.downcast().unwrap();

        let service_client = HeritageServiceClient::new(
            service_gargs.service_api_url.clone(),
            Tokens::load(&mut db)?,
        );

        let need_key_provider = match &self {
            HeirSubcmd::Create { .. }
            | HeirSubcmd::HeirConfig { .. }
            | HeirSubcmd::BackupMnemonic { .. } => true,
            _ => false,
        };

        let heir = match &self {
            HeirSubcmd::Create {
                kind,
                heir_config,
                key_provider,
                seed,
                word_count,
                with_password,
            } => {
                Heir::verify_name_is_free(&db, &heir_name)?;
                let key_provider = match key_provider {
                    KeyProviderType::None => AnyKeyProvider::None,
                    KeyProviderType::Local => {
                        let password = if *with_password {
                            Some(prompt_user_for_password(true)?)
                        } else {
                            None
                        };
                        let local_key = if let Some(seed) = seed {
                            log::info!("Restoring an heir...");
                            let mnemo = Mnemonic::parse_in(Language::English, seed.join(" "))
                                .map_err(|e| {
                                    log::error!("invalid mnemonic {e}");
                                    Error::Generic(format!("invalid mnemonic {e}"))
                                })?;
                            LocalKey::restore(mnemo, password, gargs.network)
                        } else if let Some(word_count) = word_count {
                            log::info!("Generating a new heir...");
                            LocalKey::generate(*word_count, password, gargs.network)
                        } else {
                            unreachable!("Clap ensure either seed or word_count is passed");
                        };
                        AnyKeyProvider::LocalKey(local_key)
                    } // KeyProviderType::Ledger => {
                      //     AnyKeyProvider::Ledger(LedgerKey::new(gargs.network)?)
                      // }
                };
                let heir_config = if let Some(heir_config) = heir_config {
                    match kind {
                        HeirConfigType::SinglePub => HeirConfig::SingleHeirPubkey(
                            SingleHeirPubkey::try_from(heir_config.as_str())?,
                        ),
                        HeirConfigType::Xpub => {
                            HeirConfig::HeirXPubkey(AccountXPub::try_from(heir_config.as_str())?)
                        }
                    }
                } else if !key_provider.is_none() {
                    key_provider.derive_heir_config((*kind).into())?
                } else {
                    unreachable!("clap ensures it")
                };

                let heir = Heir::new(heir_name, heir_config, key_provider)?;
                let heir = RefCell::new(heir);

                heir
            }
            _ => {
                let mut heir = Heir::load(&db, &heir_name)?;
                if need_key_provider {
                    match heir.key_provider_mut() {
                        AnyKeyProvider::None => (),
                        AnyKeyProvider::LocalKey(lk) => {
                            let password = if lk.require_password() {
                                Some(prompt_user_for_password(false)?)
                            } else {
                                None
                            };
                            lk.init_local_key(password)?;
                        }
                        AnyKeyProvider::Ledger(ledger) => ledger.init_ledger_client()?,
                    };
                }
                RefCell::new(heir)
            }
        };

        let res: Box<dyn crate::display::Displayable> = match self {
            HeirSubcmd::Create { .. } => {
                heir.borrow().create(&mut db)?;
                Box::new("Heir created")
            }
            HeirSubcmd::Rename { new_name } => {
                // First verify the destination name is free
                Heir::verify_name_is_free(&db, &new_name)?;
                // Rename
                heir.borrow_mut().db_rename(&mut db, new_name)?;
                Box::new("Heir renamed")
            }
            HeirSubcmd::Remove {
                i_understand_what_i_am_doing,
            } => {
                if !i_understand_what_i_am_doing {
                    if !heir.borrow().key_provider().is_none() {
                        ask_user_confirmation(&format!(
                            "Do you have a backup of the seed of the heir \"{}\"?",
                            heir.borrow().name()
                        ))?;
                    }
                    ask_user_confirmation(&format!(
                        "FINAL CONFIRMATION. Are you SURE you want to delete the heir \"{}\"?",
                        heir.borrow().name()
                    ))?;
                }
                heir.borrow().delete(&mut db)?;
                Box::new("Heir deleted")
            }
            HeirSubcmd::Fingerprint => Box::new(heir.borrow().fingerprint()?),
            HeirSubcmd::BackupMnemonic => Box::new(heir.borrow().backup_mnemonic()?),
            HeirSubcmd::HeirConfig => Box::new(heir.borrow().heir_config().clone()),
            HeirSubcmd::PushToService {} => todo!(),
        };
        Ok(res)
    }
}
