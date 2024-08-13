use core::{any::Any, cell::RefCell, str::FromStr};
use std::{io::Write, path::PathBuf, rc::Rc};

use btc_heritage_wallet::{
    bitcoin::{address::NetworkUnchecked, bip32::Fingerprint, psbt::Psbt, Address, Amount},
    btc_heritage::HeritageWalletBackup,
    errors::{Error, Result},
    heritage_api_client::{HeritageServiceClient, NewTx, NewTxDrainTo, NewTxRecipient, Tokens},
    online_wallet::ServiceBinding,
    AnyKeyProvider, AnyOnlineWallet, BoundFingerprint, Database, DatabaseItem, KeyProvider,
    Language, LedgerKey, LocalKey, Mnemonic, OnlineWallet, Wallet,
};
use clap::builder::{PossibleValuesParser, RangedU64ValueParser, TypedValueParser};

use crate::{
    commands::{subcmd_heir::HeirConfigType, subcmd_service_wallet},
    spendflow::SpendFlow,
    utils::{ask_user_confirmation, get_fingerprints, prompt_user_for_password},
};

use super::{
    subcmd_wallet_axpubs::WalletAXpubSubcmd, subcmd_wallet_ledger_policy::WalletLedgerPolicySubcmd,
};

/// Sub-command for wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Creates a new Heritage wallet with the chosen online-wallet and key-provider
    ///
    /// An Heritage wallet has two functional components:
    /// {n}  - The "key-provider" is the component dedicated to key management.
    /// {n}    It will be use mainly when creating a new wallet and each time you need to sign a Bitcoin transaction.
    /// {n}    Its security is critical and using a Ledger device is recommended.
    /// {n}  - The "online-wallet" is the component on which you can declare your Heritage Configuration, generate new Bitcoin addresses, synchronize with the blockchain and create new Unsigned transactions.
    Create {
        /// Specify the kind of online-wallet to use to watch the blockchain, synchronize, manage Heritage Configuration and generate addresses
        #[arg(short = 'o', long, value_name = "TYPE", aliases = ["online", "ow"], value_enum, default_value_t=OnlineWalletType::Service)]
        online_wallet: OnlineWalletType,
        /// Specify the name of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(
            long,
            value_name = "NAME",
            group = "service_bind",
            conflicts_with = "restore_backup"
        )]
        existing_service_wallet_name: Option<String>,
        /// Specify the fingerprint of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(
            long,
            value_name = "FINGERPRINT",
            group = "service_bind",
            conflicts_with = "restore_backup"
        )]
        existing_service_wallet_fingerprint: Option<Fingerprint>,
        /// Specify the ID of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(
            long,
            value_name = "WALLET_ID",
            group = "service_bind",
            conflicts_with = "restore_backup"
        )]
        existing_service_wallet_id: Option<String>,
        /// Provide a descriptors backup to restore instead of creating a new wallet from scratch
        #[arg(
            long,
            value_name = "BACKUP",
            value_parser=parse_heritage_wallet_backup,
            group = "restore_backup",
            conflicts_with = "service_bind"
        )]
        backup: Option<HeritageWalletBackup>,
        /// Provide a descriptors backup to restore instead of creating a new wallet from scratch
        #[arg(
            long,
            value_name = "PATH",
            value_hint = clap::ValueHint::FilePath,
            group = "restore_backup",
            conflicts_with = "service_bind"
        )]
        backup_file: Option<PathBuf>,
        /// Specify the kind of key-provider the wallet will use to manages secrets keys and sign transactions
        #[arg(short = 'k', long, value_name = "TYPE", aliases = ["offline", "kp"], value_enum, default_value_t=KeyProviderType::Ledger, requires_if("local", "localgen"))]
        key_provider: KeyProviderType,
        /// Disable the automatic feeding of Heritage account eXtended public keys (xpubs) to the online-wallet at creation.
        #[arg(long, visible_alias = "no-auto", default_value_t = false)]
        no_auto_feed_xpubs: bool,
        /// The mnemonic phrase to restore as a seed for the local key-provider (12, 18 or 24 words).
        #[arg(long, value_name = "WORD", num_args=2..24, group="localgen")]
        seed: Option<Vec<String>>,
        /// The length of the mnemonic phrase to generate as a seed for the local key-provider.
        #[arg(
            long, value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()),
            group="localgen"
        )]
        word_count: Option<usize>,
        /// Signal that the seed of the local key-provider should NOT be password-protected (not advised).
        #[arg(long, default_value_t = false)]
        no_password: bool,
        /// Set the Block Inclusion Objective of the wallet. It is used to compute the fee when creating a new transaction.
        #[arg(long, visible_alias = "bio", value_parser=RangedU64ValueParser::<u16>::new().range(1..=1008), default_value = "6")]
        block_inclusion_objective: u16,
    },
    /// Rename the wallet in the database to a new name
    Rename {
        new_name: String,
        /// Do not rename the wallet in the service (applicable only if online-wallet = service).
        #[arg(long, default_value_t = false)]
        local_only: bool,
    },
    /// Create a backup of the online-wallet descriptors (BIP380) that allow the restoration of your Heritage Configurations.
    /// {n}/!\ These descriptors are crucial to find and spend your bitcoins, DO NOT loose them.
    Backup {
        /// Creates a file with the descriptors backup instead of displaying them
        #[arg(long, value_name = "PATH", value_hint = clap::ValueHint::FilePath)]
        file: Option<PathBuf>,
        /// Override the file if it already exist instead of failing
        #[arg(long, default_value_t = false)]
        overwrite: bool,
    },
    /// Remove the wallet from the local database. If online-walet = service, the part in the Heritage service will stay unchanged.
    /// {n}/!\ BE AWARE THAT YOU WILL LOOSE ALL YOUR COINS IF YOUR SEED AND DESCRIPTORS ARE NOT BACKED-UP /!\
    #[command(visible_aliases = ["delete", "del"])]
    Remove,
    /// Get a new address for this wallet, based on the current Heritage Configuration
    NewAddress,
    /// List all the existing addresses for this wallet
    Addresses,
    /// List all the past transactions for this wallet
    Transactions,
    /// Commands managing the Ledger wallet policies (BIP388) of the wallet
    #[command(visible_aliases = ["ledger-policy", "lp"])]
    LedgerPolicies {
        #[command(subcommand)]
        subcmd: WalletLedgerPolicySubcmd,
    },
    /// Commands managing the Heritage configuration of the wallet
    #[command(visible_aliases = ["heritage-config", "hc"])]
    HeritageConfigs {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_heritage_config::WalletHeritageConfigSubcmd,
    },
    /// Commands managing the Account eXtended Public Keys of the wallet
    #[command(visible_aliases = ["account-xpub", "ax"])]
    AccountXpubs {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_axpubs::WalletAXpubSubcmd,
    },
    /// Sync the wallet from the Bitcoin network, updating the balance and the fee rate as needed
    Sync,
    /// Display the balance of the wallet
    #[command(visible_aliases = ["status", "stat"])]
    Balance,
    /// Display the current Block Inclusion Objective (bio) of the wallet. It is used to compute the fee when creating a new transaction.
    #[command(visible_alias = "bio")]
    BlockInclusionObjective {
        /// Set the Block Inclusion Objective of the wallet instead of showing it.
        #[arg(long, value_parser=RangedU64ValueParser::<u16>::new().range(1..=1008))]
        set: Option<u16>,
    },
    /// Display the fingerprint of the wallet
    Fingerprint,
    /// Display the mnemonic of the wallet for backup purpose
    /// {n}/!\ BE AWARE THAT THOSE INFORMATIONS WILL ALLOW SPENDING OF YOUR COINS unless the wallet is passphrase-protected /!\
    BackupMnemonic {
        #[arg(long, required = true)]
        /// Confirm that you know what you are doing
        i_understand_what_i_am_doing: bool,
    },
    /// Generate an Heir Configuration from this Heritage wallet that can be used as an heir for another Heritage wallet
    HeirConfig {
        /// The kind of Heir Configuration to generate
        #[arg(short, long, value_enum, default_value_t=HeirConfigType::Xpub)]
        kind: HeirConfigType,
    },
    /// Create a Partially Signed Bitcoin Transaction (PSBT), a.k.a an Unsigned TX, from the provided receipients information
    #[command(visible_aliases = ["send-bitcoin", "spend-bitcoins", "spend-bitcoin","sb"])]
    SendBitcoins {
        /// A recipient address and an amount to send them.
        /// {n}<AMOUNT> can be a quantity of BTC e.g. 1.0btc, 100mbtc, 100sat
        /// {n}or 'all' to drain the wallet
        #[arg(short, long, value_name="ADDRESS>:<AMOUNT", required = true, value_parser=parse_recipient)]
        recipient: Vec<(Address<NetworkUnchecked>, Option<Amount>)>,
        /// Immediately sign the PSBT
        #[arg(short, long, default_value_t = false)]
        sign: bool,
        /// Immediately broadcast the PSBT after signing it
        #[arg(short, long, default_value_t = false, requires = "sign")]
        broadcast: bool,
        /// If --sign or --broadcast are requested, do it without asking for confirmation{n}
        /// /!\ BE VERY CAREFULL with that option /!\.
        #[arg(short = 'y', long, default_value_t = false)]
        skip_confirmation: bool,
    },
    /// Sign every sign-able inputs of the given Partially Signed Bitcoin Transaction (PSBT)
    #[command(visible_alias = "sign")]
    SignPsbt {
        /// The PSBT to sign
        psbt: Psbt,
        /// Immediately broadcast the PSBT after signing it
        #[arg(short, long, default_value_t = false)]
        broadcast: bool,
        /// If --broadcast is requested, do it without asking for confirmation{n}
        /// /!\ BE VERY CAREFULL with that option /!\.
        #[arg(short = 'y', long, default_value_t = false)]
        skip_confirmation: bool,
    },
    /// Extract a raw transaction from the given Partially Signed Bitcoin Transaction (PSBT) and broadcast it to the Bitcoin network
    #[command(visible_alias = "broadcast")]
    BroadcastPsbt {
        /// The PSBT to broadcast. Must have every inputs correctly signed for this to work.
        psbt: Psbt,
    },
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OnlineWalletType {
    /// No online wallet, the resulting wallet will not be able to sync, generate addresses, etc... (it will be sign-only)
    None,
    /// Use the Heritage service as the online wallet
    Service,
    /// Use an Electrum server as the online wallet
    Electrum,
    /// Use a Bitcoin node as the online wallet
    Bitcoin,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum KeyProviderType {
    /// No key provider, the resulting wallet will not be able to sign transactions (it will be watch-only)
    None,
    /// Store the seed in the local database (discouraged unless you know what you do. Please use a password to protect the seed)
    Local,
    /// Use a Ledger hardware-wallet device
    Ledger,
}

impl super::CommandExecutor for WalletSubcmd {
    fn execute(mut self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (mut db, wallet_name, gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            Database,
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();

        let service_client =
            HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&db)?);

        // TODO
        let _bitcoin_client = (electrum_gargs, bitcoinrpc_gargs);

        let need_online_wallet = match &self {
            WalletSubcmd::Create { .. }
            | WalletSubcmd::Backup { .. }
            | WalletSubcmd::Sync
            | WalletSubcmd::Balance
            | WalletSubcmd::SendBitcoins { .. }
            | WalletSubcmd::BroadcastPsbt { .. }
            | WalletSubcmd::BlockInclusionObjective { .. }
            | WalletSubcmd::Addresses
            | WalletSubcmd::Transactions
            | WalletSubcmd::NewAddress
            | WalletSubcmd::HeritageConfigs { .. } => true,
            WalletSubcmd::SignPsbt { broadcast, .. } if *broadcast => true,
            WalletSubcmd::Rename { local_only, .. } if !*local_only => true,
            WalletSubcmd::LedgerPolicies { subcmd } => match subcmd {
                WalletLedgerPolicySubcmd::List | WalletLedgerPolicySubcmd::AutoRegister => true,
                WalletLedgerPolicySubcmd::ListRegistered
                | WalletLedgerPolicySubcmd::Register { .. } => false,
            },
            WalletSubcmd::AccountXpubs { subcmd } => match subcmd {
                WalletAXpubSubcmd::AutoAdd { .. }
                | WalletAXpubSubcmd::ListAdded { .. }
                | WalletAXpubSubcmd::Add { .. } => true,
                WalletAXpubSubcmd::Generate { .. } => false,
            },
            WalletSubcmd::Remove
            | WalletSubcmd::SignPsbt { .. }
            | WalletSubcmd::Rename { .. }
            | WalletSubcmd::Fingerprint
            | WalletSubcmd::BackupMnemonic { .. }
            | WalletSubcmd::HeirConfig { .. } => false,
        };
        let need_key_provider = match &self {
            WalletSubcmd::Create { .. }
            | WalletSubcmd::SignPsbt { .. }
            | WalletSubcmd::BackupMnemonic { .. }
            | WalletSubcmd::HeirConfig { .. } => true,
            WalletSubcmd::LedgerPolicies { subcmd } => match subcmd {
                WalletLedgerPolicySubcmd::ListRegistered
                | WalletLedgerPolicySubcmd::AutoRegister
                | WalletLedgerPolicySubcmd::Register { .. } => true,
                WalletLedgerPolicySubcmd::List => false,
            },
            WalletSubcmd::AccountXpubs { subcmd } => match subcmd {
                WalletAXpubSubcmd::Generate { .. } | WalletAXpubSubcmd::AutoAdd { .. } => true,
                WalletAXpubSubcmd::ListAdded { .. } | WalletAXpubSubcmd::Add { .. } => false,
            },
            WalletSubcmd::SendBitcoins { sign, .. } if *sign => true,
            WalletSubcmd::Rename { .. }
            | WalletSubcmd::SendBitcoins { .. }
            | WalletSubcmd::Backup { .. }
            | WalletSubcmd::Remove
            | WalletSubcmd::NewAddress
            | WalletSubcmd::Addresses
            | WalletSubcmd::Transactions
            | WalletSubcmd::HeritageConfigs { .. }
            | WalletSubcmd::Sync
            | WalletSubcmd::Balance
            | WalletSubcmd::BlockInclusionObjective { .. }
            | WalletSubcmd::Fingerprint
            | WalletSubcmd::BroadcastPsbt { .. } => false,
        };

        let wallet = match &mut self {
            WalletSubcmd::Create {
                online_wallet,
                existing_service_wallet_name,
                existing_service_wallet_fingerprint,
                existing_service_wallet_id,
                backup,
                backup_file,
                key_provider,
                no_auto_feed_xpubs,
                seed,
                word_count,
                no_password,
                block_inclusion_objective,
            } => {
                Wallet::verify_name_is_free(&db, &wallet_name)?;
                let backup = if let Some(backup_file) = backup_file {
                    Some(parse_heritage_wallet_backup(
                        &std::fs::read_to_string(backup_file.as_path()).map_err(Error::generic)?,
                    )?)
                } else {
                    backup.take()
                };
                let key_provider = match key_provider {
                    KeyProviderType::None => AnyKeyProvider::None,
                    KeyProviderType::Local => {
                        let password = if *no_password {
                            None
                        } else {
                            Some(prompt_user_for_password(true)?)
                        };
                        let local_key = if let Some(seed) = seed {
                            log::info!("Restoring a wallet...");
                            let mnemo = Mnemonic::parse_in(Language::English, seed.join(" "))
                                .map_err(|e| {
                                    log::error!("invalid mnemonic {e}");
                                    Error::Generic(format!("invalid mnemonic {e}"))
                                })?;
                            LocalKey::restore(mnemo, password, gargs.network)
                        } else if let Some(word_count) = word_count {
                            log::info!("Generating a new wallet...");
                            LocalKey::generate(*word_count, password, gargs.network)
                        } else {
                            unreachable!("Clap ensure either seed or word_count is passed");
                        };
                        AnyKeyProvider::LocalKey(local_key)
                    }
                    KeyProviderType::Ledger => {
                        AnyKeyProvider::Ledger(LedgerKey::new(gargs.network)?)
                    }
                };
                let online_wallet = match online_wallet {
                    OnlineWalletType::None => AnyOnlineWallet::None,
                    OnlineWalletType::Service => AnyOnlineWallet::Service(
                        if let Some(wallet_name) = existing_service_wallet_name {
                            ServiceBinding::bind_by_name(
                                wallet_name,
                                service_client,
                                gargs.network,
                            )?
                        } else if let Some(fingerprint) = existing_service_wallet_fingerprint {
                            ServiceBinding::bind_by_fingerprint(
                                *fingerprint,
                                service_client,
                                gargs.network,
                            )?
                        } else if let Some(wallet_id) = existing_service_wallet_id {
                            ServiceBinding::bind_by_id(&wallet_id, service_client, gargs.network)?
                        } else {
                            ServiceBinding::create(
                                &wallet_name,
                                backup,
                                *block_inclusion_objective,
                                service_client,
                                gargs.network,
                            )?
                        },
                    ),
                    OnlineWalletType::Electrum => todo!(),
                    OnlineWalletType::Bitcoin => todo!(),
                };
                let wallet = Wallet::new(wallet_name, key_provider, online_wallet)?;
                let wallet = Rc::new(RefCell::new(wallet));

                // Auto-feed
                if !(*no_auto_feed_xpubs
                    || wallet.as_ref().borrow().key_provider().is_none()
                    || wallet.as_ref().borrow().online_wallet().is_none())
                {
                    (WalletAXpubSubcmd::AutoAdd { count: 20 }).execute(Box::new(wallet.clone()))?;
                }
                wallet
            }
            _ => {
                let mut wallet = Wallet::load(&db, &wallet_name)?;
                if need_key_provider {
                    match wallet.key_provider_mut() {
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
                if need_online_wallet {
                    match wallet.online_wallet_mut() {
                        AnyOnlineWallet::None => (),
                        AnyOnlineWallet::Service(sb) => sb.init_service_client(service_client)?,
                        AnyOnlineWallet::Local(_) => todo!(),
                    };
                }
                Rc::new(RefCell::new(wallet))
            }
        };

        let wallet_ref = wallet.as_ref();

        let res: Box<dyn crate::display::Displayable> = match self {
            WalletSubcmd::Create { .. } => {
                wallet_ref.borrow().create(&mut db)?;
                Box::new("Wallet created")
            }
            WalletSubcmd::Rename {
                new_name,
                local_only,
            } => {
                // First verify the destination name is free
                Wallet::verify_name_is_free(&db, &new_name)?;
                if let AnyOnlineWallet::Service(sb) = wallet.borrow().online_wallet() {
                    if !local_only {
                        let cmd = subcmd_service_wallet::WalletSubcmd::Update {
                            name: Some(new_name.clone()),
                            block_inclusion_objective: None,
                        };
                        cmd.execute(Box::new((
                            sb.wallet_id().to_owned(),
                            sb.service_client().clone(),
                        )))?;
                    }
                };
                // Rename
                wallet.borrow_mut().db_rename(&mut db, new_name)?;
                Box::new("Wallet renamed")
            }
            WalletSubcmd::Backup {
                file,
                overwrite: override_content,
            } => {
                let backup = wallet.borrow().online_wallet().backup_descriptors()?;
                if let Some(path) = file {
                    let mut file = if override_content {
                        std::fs::File::create(path)
                    } else {
                        std::fs::File::create_new(path)
                    }
                    .map_err(Error::generic)?;
                    file.write_all(serde_json::to_string_pretty(&backup)?.as_bytes())
                        .map_err(Error::generic)?;
                    Box::new("Backup created")
                } else {
                    Box::new(backup)
                }
            }
            WalletSubcmd::Remove => {
                {
                    let wallet = wallet_ref.borrow();
                    let wallet_name = wallet.name();
                    if !wallet.key_provider().is_none() && !wallet.key_provider().is_ledger() {
                        if !ask_user_confirmation(&format!(
                            "Do you have a backup of the seed of the wallet \"{wallet_name}\"?"
                        ))? {
                            return Ok(Box::new("Delete wallet cancelled"));
                        }
                    }
                    if !wallet.online_wallet().is_none() {
                        if !ask_user_confirmation(&format!(
                            "Do you have a backup of the descriptors of \
                            the wallet \"{wallet_name}\"?"
                        ))? {
                            return Ok(Box::new("Delete wallet cancelled"));
                        }
                    }
                    if !ask_user_confirmation(
                        &"Do you understand that *BOTH* the seed and the descriptors \
                        are necessary to re-access bitcoins in an Heritage wallet?",
                    )? {
                        return Ok(Box::new("Delete wallet cancelled"));
                    }
                    if !ask_user_confirmation(&format!(
                        "FINAL CONFIRMATION. Are you 100% SURE you want to \
                        delete the wallet \"{wallet_name}\"?"
                    ))? {
                        return Ok(Box::new("Delete wallet cancelled"));
                    }
                }
                wallet_ref.borrow_mut().delete(&mut db)?;
                Box::new("Wallet deleted")
            }
            WalletSubcmd::NewAddress => Box::new(wallet.borrow().online_wallet().get_address()?),
            WalletSubcmd::Addresses => Box::new(wallet.borrow().online_wallet().list_addresses()?),
            WalletSubcmd::Transactions => {
                Box::new(wallet.borrow().online_wallet().list_transactions()?)
            }
            WalletSubcmd::LedgerPolicies { subcmd } => {
                subcmd.execute(Box::new((wallet.clone(), db)))?
            }
            WalletSubcmd::HeritageConfigs { subcmd } => {
                subcmd.execute(Box::new((wallet.clone(), db)))?
            }
            WalletSubcmd::AccountXpubs { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::Sync => {
                wallet.borrow_mut().sync()?;
                Box::new("Synchronization done")
            }
            WalletSubcmd::Balance => Box::new(wallet_ref.borrow().get_wallet_status()?),
            WalletSubcmd::BlockInclusionObjective { set } => {
                let wallet_status = if let Some(bio) = set {
                    wallet_ref.borrow_mut().set_block_inclusion_objective(bio)?
                } else {
                    wallet_ref.borrow().get_wallet_status()?
                };
                Box::new(wallet_status.block_inclusion_objective)
            }
            WalletSubcmd::Fingerprint => Box::new(wallet_ref.borrow().fingerprint()?),
            WalletSubcmd::BackupMnemonic {
                i_understand_what_i_am_doing: _,
            } => Box::new(wallet_ref.borrow().backup_mnemonic()?),
            WalletSubcmd::HeirConfig { kind } => {
                Box::new(wallet_ref.borrow().derive_heir_config(kind.into())?)
            }
            WalletSubcmd::SendBitcoins {
                recipient,
                sign,
                broadcast,
                skip_confirmation,
            } => {
                // Check every addresses against the Network
                let recipient = recipient
                    .into_iter()
                    .map(|(ad, am)| {
                        Ok((
                            ad.require_network(gargs.network)
                                .map_err(|e| Error::InvalidAddressNetwork(e.to_string()))?,
                            am,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                // All recipients have an amount
                // OR
                // There is only one recipient
                let spending_config = if recipient.iter().all(|(_, a)| a.is_some()) {
                    NewTx::Recipients(
                        recipient
                            .into_iter()
                            .map(|(address, amount)| NewTxRecipient {
                                address: address.to_string(),
                                amount: amount.expect("we verified every amount is some").to_sat(),
                            })
                            .collect(),
                    )
                } else if recipient.len() == 1 {
                    NewTx::DrainTo(NewTxDrainTo {
                        drain_to: recipient[0].0.to_string(),
                    })
                } else {
                    log::error!("Exactly one recipient is allowed when using amount 'all'");
                    return Err(Error::Generic(
                        "Exactly one recipient is allowed when using amount 'all'".to_owned(),
                    ));
                };

                let wallet = wallet_ref.borrow();
                // Get the PSBT
                let (psbt, summary) = wallet.create_psbt(spending_config)?;
                SpendFlow::new(psbt, gargs.network)
                    .transaction_summary(&summary)
                    .fingerprints(&get_fingerprints(&db)?)
                    .display()
                    .set_sign(if sign {
                        Some(wallet.key_provider())
                    } else {
                        None
                    })
                    .set_broadcast(if broadcast {
                        Some(wallet.online_wallet())
                    } else {
                        None
                    })
                    .set_skip_confirmations(skip_confirmation)
                    .run()?
            }
            WalletSubcmd::SignPsbt {
                psbt,
                broadcast,
                skip_confirmation,
            } => {
                let wallet = wallet_ref.borrow();
                SpendFlow::new(psbt, gargs.network)
                    .fingerprints(&get_fingerprints(&db)?)
                    .sign(wallet.key_provider())
                    .set_skip_confirmations(skip_confirmation)
                    .set_broadcast(if broadcast {
                        Some(wallet.online_wallet())
                    } else {
                        None
                    })
                    .run()?
            }
            WalletSubcmd::BroadcastPsbt { psbt } => {
                SpendFlow::<AnyKeyProvider, _>::new(psbt, gargs.network)
                    .broadcast(wallet_ref.borrow().online_wallet())
                    .run()?
            }
        };
        Ok(res)
    }
}

fn parse_heritage_wallet_backup(
    val: &str,
) -> core::result::Result<HeritageWalletBackup, serde_json::Error> {
    serde_json::from_str(val)
}

fn parse_recipient(val: &str) -> Result<(Address<NetworkUnchecked>, Option<Amount>)> {
    if !val.contains(':') {
        return Err(Error::Generic(
            "invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned(),
        ));
    }

    let mut parts = val.split(':');
    let addr = parts.next().ok_or_else(|| {
        Error::Generic("invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned())
    })?;
    let addr = Address::from_str(addr).map_err(Error::generic)?;

    let amount = parts.next().ok_or_else(|| {
        Error::Generic("invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned())
    })?;
    let amount = match amount {
        "all" => None,
        _ => Some(amount.parse::<Amount>().map_err(Error::generic)?),
    };

    if parts.next().is_some() {
        return Err(Error::Generic(
            "invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned(),
        ));
    }

    Ok((addr, amount))
}
