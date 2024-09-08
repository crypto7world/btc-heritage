use core::{any::Any, cell::RefCell};

use btc_heritage_wallet::{
    btc_heritage::{AccountXPub, HeirConfig, SingleHeirPubkey},
    errors::{Error, Result},
    heritage_service_api_client::{
        EmailAddress, HeirContact, HeirCreate, HeirPermission, HeirPermissions,
        HeritageServiceClient, MainContact, Tokens,
    },
    AnyKeyProvider, BoundFingerprint, Database, DatabaseItem, Heir, KeyProvider, Language,
    LocalKey, Mnemonic,
};
use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::utils::{ask_user_confirmation, prompt_user_for_password};

use super::{subcmd_service_heir::CliHeirPermission, subcmd_wallet::KeyProviderType};

/// Sub-command for heirs.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum HeirSubcmd {
    /// Declare a new heir, optionnaly exporting it to the Heritage Service
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
            requires_ifs=[("none", "heir_config")]
        )]
        key_provider: KeyProviderType,
        /// The mnemonic phrase to restore as a seed for the local key-provider (12, 18 or 24 words).
        #[arg(long, value_name = "WORD", num_args=2..=24, group="localgen")]
        seed: Option<Vec<String>>,
        /// The length of the mnemonic phrase to generate as a seed for the local key-provider.
        #[arg(
            long, value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()),
            group="localgen", default_value="12"
        )]
        word_count: usize,
        /// Signal that the seed of the local key-provider should be password-protected.
        #[arg(long, default_value_t = false)]
        with_password: bool,
        /// Also create the heir in the Heritage service. You must specify at least one email address for this to work.
        /// {n}This just ease heir declaration in the service, further management of heirs on the
        /// service side can be done with the "service heir <...>" command familly.
        #[arg(long, default_value_t = false, requires = "email")]
        export: bool,
        /// Use this email address as main contact when creating the heir in the service (implies "--export").
        /// {n}Can be used multiple times, additional emails will be additionnal contacts
        #[arg(long)]
        email: Option<Vec<String>>,
        /// Add a custom message to include in communications with the heir (implies "--export")
        #[arg(long, requires = "email")]
        custom_message: Option<String>,
        /// The permissions of the heir (implies "--export"). [default: [OwnerEmail]]
        #[arg(long, visible_alias="perms", value_delimiter=',', value_enum, num_args=1.., requires = "email")]
        permissions: Option<Vec<CliHeirPermission>>,
    },
    /// Rename the heir in the database to a new name
    Rename { new_name: String },
    /// Remove the heir from the database
    /// {n}/!\ BE AWARE THAT YOU WILL NOT BE ABLE TO RETRIEVE THE SEED IF IT IS NOT BACKED-UP /!\
    #[command(visible_aliases = ["delete", "del"])]
    Remove {
        #[arg(long)]
        /// Confirm that you know what you are doing and skips verification prompts
        i_understand_what_i_am_doing: bool,
    },
    /// Try to create the heir on the Heritage service. Will fail if the heir already exist.
    /// To manage an existing heir, use the "service heir <...>" command familly
    Export {
        /// Use this email address as main contact when creating the heir in the service.
        /// {n}Can be used multiple times, additional emails will be additionnal contacts
        #[arg(long, required = true)]
        email: Vec<String>,
        /// Add a custom message to include in communications with the heir
        #[arg(long)]
        custom_message: Option<String>,
        /// The permissions of the heir. [default: [OwnerEmail]]
        #[arg(long, visible_alias="perms", value_delimiter=',', value_enum, num_args=1..)]
        permissions: Option<Vec<CliHeirPermission>>,
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

impl super::CommandExecutor for HeirSubcmd {
    fn execute(mut self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (mut db, heir_name, gargs, service_gargs, _electrum_gargs, _bitcoinrpc_gargs): (
            Database,
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();

        let service_client =
            HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?);

        let need_key_provider = match &self {
            HeirSubcmd::Create { .. } | HeirSubcmd::BackupMnemonic { .. } => true,
            HeirSubcmd::Rename { .. }
            | HeirSubcmd::HeirConfig { .. }
            | HeirSubcmd::Remove { .. }
            | HeirSubcmd::Export { .. }
            | HeirSubcmd::Fingerprint => false,
        };

        let heir = match &mut self {
            HeirSubcmd::Create {
                kind,
                heir_config,
                key_provider,
                seed,
                word_count,
                with_password,
                export: _,
                email,
                custom_message,
                permissions,
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
                            log::info!("Restoring an heir from a seed...");
                            let mnemo = Mnemonic::parse_in(Language::English, seed.join(" "))
                                .map_err(|e| {
                                    log::error!("invalid mnemonic {e}");
                                    Error::Generic(format!("invalid mnemonic {e}"))
                                })?;
                            LocalKey::restore(mnemo, password, gargs.network)
                        } else {
                            log::info!("Generating a new heir...");
                            LocalKey::generate(*word_count, password, gargs.network)
                        };
                        AnyKeyProvider::LocalKey(local_key)
                    }
                    KeyProviderType::Ledger => todo!(),
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

                if email.is_some() {
                    create_heir_in_service(
                        heir_name.clone(),
                        heir_config.clone(),
                        email.take().unwrap(),
                        custom_message.take(),
                        permissions.take(),
                        &service_client,
                    )?;
                }

                let heir = Heir::new(heir_name, heir_config, key_provider);
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
                        if !ask_user_confirmation(&format!(
                            "Do you have a backup of the seed of the heir \"{}\"?",
                            heir.borrow().name()
                        ))? {
                            return Ok(Box::new("Delete heir-wallet cancelled"));
                        }
                    }
                    if !ask_user_confirmation(&format!(
                        "FINAL CONFIRMATION. Are you SURE you want to delete the heir \"{}\"?",
                        heir.borrow().name()
                    ))? {
                        return Ok(Box::new("Delete heir-wallet cancelled"));
                    }
                }
                heir.borrow().delete(&mut db)?;
                Box::new("Heir deleted")
            }
            HeirSubcmd::Export {
                email,
                custom_message,
                permissions,
            } => {
                let h = heir.borrow();
                create_heir_in_service(
                    h.name().to_owned(),
                    h.heir_config.clone(),
                    email,
                    custom_message,
                    permissions,
                    &service_client,
                )?;
                Box::new("Heir exported")
            }
            HeirSubcmd::Fingerprint => Box::new(heir.borrow().fingerprint()?),
            HeirSubcmd::BackupMnemonic => Box::new(heir.borrow().backup_mnemonic()?),
            HeirSubcmd::HeirConfig => Box::new(heir.into_inner().heir_config),
        };
        Ok(res)
    }
}

fn create_heir_in_service(
    display_name: String,
    heir_config: HeirConfig,
    mut emails: Vec<String>,
    custom_message: Option<String>,
    permissions: Option<Vec<CliHeirPermission>>,
    service_client: &HeritageServiceClient,
) -> Result<()> {
    log::debug!(
        "create_heir - display_name={display_name} heir_config={heir_config:?} \
    emails={emails:?} custom_message={} permissions={permissions:?}",
        custom_message.is_some()
    );
    let heir_create = HeirCreate {
        display_name,
        heir_config,
        main_contact: MainContact {
            email: EmailAddress::try_from(emails.remove(0)).map_err(|e| Error::Generic(e))?,
            custom_message: custom_message,
        },
        permissions: permissions
            .map(|vhp| HeirPermissions::from(vhp.into_iter().map(|cli_hp| cli_hp.into())))
            .unwrap_or(HeirPermissions::from([HeirPermission::OwnerEmail])),
    };
    let h = service_client.post_heirs(heir_create)?;
    if emails.len() > 0 {
        service_client.post_heir_contacts(
            &h.id,
            emails
                .into_iter()
                .map(|c| {
                    Ok(HeirContact::Email {
                        email: EmailAddress::try_from(c).map_err(|e| Error::Generic(e))?,
                    })
                })
                .collect::<Result<_>>()?,
        )?;
    }
    Ok(())
}
