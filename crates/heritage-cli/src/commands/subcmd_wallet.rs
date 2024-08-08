use core::{any::Any, cell::RefCell, str::FromStr};
use std::rc::Rc;

use btc_heritage_wallet::{
    bitcoin::{address::NetworkUnchecked, bip32::Fingerprint, psbt::Psbt, Address, Amount},
    errors::{Error, Result},
    heritage_api_client::{HeritageServiceClient, NewTx, NewTxDrainTo, NewTxRecipient, Tokens},
    wallet_online::ServiceBinding,
    AnyKeyProvider, AnyWalletOnline, BoundFingerprint, Database, DatabaseItem, KeyProvider,
    Language, LedgerKey, LocalKey, Mnemonic, Wallet, WalletOnline,
};
use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::{
    spendflow::SpendFlow,
    utils::{ask_user_confirmation, get_fingerprints, prompt_user_for_password},
};

use super::{
    subcmd_wallet_axpubs::WalletAXpubSubcmd, subcmd_wallet_ledger_policy::WalletLedgerPolicySubcmd,
};

/// Sub-command for wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Creates a new Heritage wallet with the chosen online-wallet and offline key-provider
    Create {
        /// Specify the kind of online-wallet to use to watch the blockchain, synchronize, manage Heritage Configuration and generate addresses
        #[arg(short = 'o', long, value_name = "TYPE", aliases = ["online", "ow"], value_enum, default_value_t=OnlineWalletType::Service)]
        online_wallet: OnlineWalletType,
        /// Specify the name of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(long, value_name = "NAME", conflicts_with_all=["existing_service_wallet_fingerprint", "existing_service_wallet_id"])]
        existing_service_wallet_name: Option<String>,
        /// Specify the fingerprint of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(long, value_name = "FINGERPRINT", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_id"])]
        existing_service_wallet_fingerprint: Option<Fingerprint>,
        /// Specify the ID of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_wallet = service)
        #[arg(long, value_name = "WALLET_ID", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_fingerprint"])]
        existing_service_wallet_id: Option<String>,
        /// Specify the kind of key-provider the wallet will use to manages secrets keys and sign transactions
        #[arg(short = 'k', long, value_name = "TYPE", aliases = ["offline", "kp"], value_enum, default_value_t=KeyProviderType::Ledger, requires_if("local", "localgen"))]
        key_provider: KeyProviderType,
        /// Disable the automatic feeding of Heritage account eXtended public keys (xpubs) to the online-wallet at creation.
        #[arg(long, default_value_t = false)]
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
    },
    /// Rename the wallet in the database to a new name
    Rename { new_name: String },
    /// Remove the wallet from the database
    /// {n}/!\ BE AWARE THAT YOU WILL LOOSE ALL YOUR COINS IF YOUR SEED AND DESCRIPTORS ARE NOT BACKED-UP /!\
    Remove {
        #[arg(long)]
        /// Confirm that you know what you are doing and skips verification prompts
        i_understand_what_i_am_doing: bool,
    },
    /// Commands related to the Bitcoin addresses of the wallet
    Addresses {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_addresses::WalletAddressesSubcmd,
    },
    /// Commands managing the Descriptors (BIP380) of the wallet
    Descriptors {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_descriptors::WalletDescriptorsSubcmd,
    },
    /// Commands managing the Ledger wallet policies (BIP388) of the wallet
    LedgerPolicies {
        #[command(subcommand)]
        subcmd: WalletLedgerPolicySubcmd,
    },
    /// Commands managing the Heritage configuration of the wallet
    HeritageConfigs {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_heritage_config::WalletHeritageConfigSubcmd,
    },
    /// Commands managing the Account eXtended Public Keys of the wallet
    AccountXpubs {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_axpubs::WalletAXpubSubcmd,
    },
    /// Sync the wallet from the Bitcoin network
    Sync,
    /// Display the balance of the wallet
    Balance,
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
    SignPsbt {
        /// The PSBT to sign
        psbt: Psbt,
        /// Immediately broadcast the PSBT after signing it
        #[arg(long, default_value_t = false)]
        broadcast: bool,
        /// If --broadcast is requested, do it without asking for confirmation{n}
        /// /!\ BE VERY CAREFULL with that option /!\.
        #[arg(short = 'y', long, default_value_t = false)]
        skip_confirmation: bool,
    },
    /// Extract a raw transaction from the given Partially Signed Bitcoin Transaction (PSBT) and broadcast it to the Bitcoin network
    BroadcastPsbt {
        /// The PSBT to broadcast. Must have every inputs correctly signed for this to work.
        psbt: Psbt,
    },
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
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (mut db, wallet_name, gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            Database,
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();

        let service_client =
            HeritageServiceClient::new(service_gargs.service_api_url.clone(), Tokens::load(&db)?);

        // TODO
        let _bitcoin_client = (electrum_gargs, bitcoinrpc_gargs);

        let need_online_wallet = match &self {
            WalletSubcmd::Create { .. }
            | WalletSubcmd::Descriptors { .. }
            | WalletSubcmd::Sync
            | WalletSubcmd::Balance
            | WalletSubcmd::SendBitcoins { .. }
            | WalletSubcmd::BroadcastPsbt { .. }
            | WalletSubcmd::Addresses { .. }
            | WalletSubcmd::HeritageConfigs { .. } => true,
            WalletSubcmd::SignPsbt { broadcast, .. } if *broadcast => true,
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
            _ => false,
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
            _ => false,
        };

        let wallet = match &self {
            WalletSubcmd::Create {
                online_wallet,
                existing_service_wallet_name,
                existing_service_wallet_fingerprint,
                existing_service_wallet_id,
                key_provider,
                no_auto_feed_xpubs,
                seed,
                word_count,
                no_password,
            } => {
                Wallet::verify_name_is_free(&db, &wallet_name)?;
                let online_wallet = match online_wallet {
                    OnlineWalletType::None => AnyWalletOnline::None,
                    OnlineWalletType::Service => AnyWalletOnline::Service(
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
                            ServiceBinding::create(&wallet_name, service_client, gargs.network)?
                        },
                    ),
                    OnlineWalletType::Electrum => todo!(),
                    OnlineWalletType::Bitcoin => todo!(),
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
                        AnyWalletOnline::None => (),
                        AnyWalletOnline::Service(sb) => sb.init_service_client(service_client)?,
                        AnyWalletOnline::Local(_) => todo!(),
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
            WalletSubcmd::Rename { new_name } => {
                // First verify the destination name is free
                Wallet::verify_name_is_free(&db, &new_name)?;
                // Rename
                wallet.borrow_mut().db_rename(&mut db, new_name)?;
                Box::new("Wallet renamed")
            }
            WalletSubcmd::Remove {
                i_understand_what_i_am_doing,
            } => {
                if !i_understand_what_i_am_doing {
                    let wallet = wallet_ref.borrow();
                    let wallet_name = wallet.name();
                    if !wallet.key_provider().is_none() {
                        ask_user_confirmation(&format!(
                            "Do you have a backup of the seed of the wallet \"{wallet_name}\"?"
                        ))?;
                    }
                    if !wallet.online_wallet().is_none() {
                        ask_user_confirmation(&format!(
                            "Do you have a backup of the descriptors of \
                            the wallet \"{wallet_name}\"?"
                        ))?;
                    }
                    ask_user_confirmation(
                        &"Do you understand that *BOTH* the seed and the descriptors \
                        are necessary to re-access bitcoins in an Heritage wallet?",
                    )?;
                    ask_user_confirmation(&format!(
                        "FINAL CONFIRMATION. Are you 100% SURE you want to \
                        delete the wallet \"{wallet_name}\"?"
                    ))?;
                }
                wallet_ref.borrow_mut().delete(&mut db)?;
                Box::new("Wallet deleted")
            }
            WalletSubcmd::Addresses { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::Descriptors { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::LedgerPolicies { subcmd } => {
                let res = subcmd.execute(Box::new((wallet.clone(), db)))?;
                res
            }
            WalletSubcmd::HeritageConfigs { subcmd } => {
                let service_client = HeritageServiceClient::new(
                    service_gargs.service_api_url.clone(),
                    Tokens::load(&db)?,
                );
                subcmd.execute(Box::new((wallet.clone(), db, service_client)))?
            }
            WalletSubcmd::AccountXpubs { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::Sync => {
                wallet.borrow_mut().sync()?;
                Box::new("Synchronization done")
            }
            WalletSubcmd::Balance => Box::new(wallet_ref.borrow().get_wallet_info()?),
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
    let addr = Address::from_str(addr).map_err(|e| Error::Generic(e.to_string()))?;

    let amount = parts.next().ok_or_else(|| {
        Error::Generic("invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned())
    })?;
    let amount = match amount {
        "all" => None,
        _ => Some(
            amount
                .parse::<Amount>()
                .map_err(|e| Error::Generic(e.to_string()))?,
        ),
    };

    if parts.next().is_some() {
        return Err(Error::Generic(
            "invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned(),
        ));
    }

    Ok((addr, amount))
}
