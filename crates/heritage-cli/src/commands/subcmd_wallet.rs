use std::{any::Any, cell::RefCell, rc::Rc, str::FromStr};

use btc_heritage_wallet::{
    bitcoin::{bip32::Fingerprint, psbt::Psbt, Address, Amount},
    errors::{Error, Result},
    AnyWalletOffline, AnyWalletOnline, Database, HeritageServiceClient, LedgerKey, NewTx,
    NewTxRecipient, ServiceBinding, Tokens, Wallet, WalletOffline, WalletOnline,
};
use clap::builder::{PossibleValuesParser, TypedValueParser};

use crate::utils::ask_user_confirmation;

use super::{
    subcmd_wallet_axpubs::WalletAXpubSubcmd, subcmd_wallet_ledger_policy::WalletLedgerPolicySubcmd,
};

/// Sub-command for wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Creates a new Heritage wallet with the chosen online and offline components
    Create {
        /// Specify the kind of online-component the wallet will use
        #[arg(long, value_name = "TYPE", alias = "online", value_enum, default_value_t=OnlineComponentType::Service)]
        online_component: OnlineComponentType,
        /// Specify the name of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        #[arg(long, value_name = "NAME", conflicts_with_all=["existing_service_wallet_fingerprint", "existing_service_wallet_id"])]
        existing_service_wallet_name: Option<String>,
        /// Specify the fingerprint of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        #[arg(long, value_name = "FINGERPRINT", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_id"])]
        existing_service_wallet_fingerprint: Option<Fingerprint>,
        /// Specify the ID of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        #[arg(long, value_name = "WALLET_ID", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_fingerprint"])]
        existing_service_wallet_id: Option<String>,
        /// Specify the kind of offline-component the wallet will use
        #[arg(long, value_name = "TYPE", alias = "offline", value_enum, default_value_t=OfflineComponentType::Ledger, requires_if("local", "localgen"))]
        offline_component: OfflineComponentType,
        /// Automaticly feed Heritage account eXtended public keys (xpubs) to the online component of the wallet if possible.
        #[arg(long, default_value_t = true)]
        auto_feed_xpubs: bool,
        /// The mnemonic phrase to restore as a seed for the Heritage wallet (12, 18 or 24 words) (if offline_component = local).
        #[arg(long, value_name = "WORD", num_args=2..24, group="localgen")]
        seed: Option<Vec<String>>,
        /// The length of the mnemonic phrase to generate as a seed for the Heritage wallet (if offline_component = local).
        #[arg(
            long, value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()),
            group="localgen"
        )]
        word_count: Option<usize>,
        /// Signal that the seed is password-protected (if offline_component = local).
        #[arg(short = 'p', long, default_value_t = true)]
        with_password: bool,
    },
    /// Remove the wallet from the database
    Remove,
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
    /// Display info about of the wallet, like the balance
    Info,
    /// Generate a new Bitcoin address for the Heritage wallet
    GetAddress,
    /// Create a Partially Signed Bitcoin Transaction (PSBT), a.k.a an Unsigned TX, from the provided receipients information
    SendBitcoins {
        /// A recipient for our BTC
        #[arg(short, long, value_name="ADDRESS>:<AMOUNT", required = true, value_parser=parse_recipient)]
        recipient: Vec<(Address, Amount)>,
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
    /// Display infos on the given Partially Signed Bitcoin Transaction (PSBT)
    DisplayPsbt {
        /// The PSBT to display.
        psbt: Psbt,
    },
}

impl super::CommandExecutor for WalletSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (wallet_name, gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();
        let mut db = Database::new(&gargs.datadir, gargs.network)?;

        let service_client =
            HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?);

        let need_online_component = match &self {
            WalletSubcmd::Create { .. }
            | WalletSubcmd::Descriptors { .. }
            | WalletSubcmd::Sync
            | WalletSubcmd::Info
            | WalletSubcmd::GetAddress
            | WalletSubcmd::SendBitcoins { .. }
            | WalletSubcmd::BroadcastPsbt { .. }
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
        let need_offline_component = match &self {
            WalletSubcmd::Create { .. } | WalletSubcmd::SignPsbt { .. } => true,
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
                online_component,
                existing_service_wallet_name,
                existing_service_wallet_fingerprint,
                existing_service_wallet_id,
                offline_component,
                auto_feed_xpubs,
                seed,
                word_count,
                with_password,
            } => {
                Wallet::verify_name_is_free(&db, &wallet_name)?;
                let online_wallet = match online_component {
                    OnlineComponentType::None => AnyWalletOnline::None,
                    OnlineComponentType::Service => AnyWalletOnline::Service(
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
                    OnlineComponentType::Electrum => todo!(),
                    OnlineComponentType::Bitcoin => todo!(),
                };
                let offline_wallet = match offline_component {
                    OfflineComponentType::None => AnyWalletOffline::None,
                    OfflineComponentType::Local => todo!(),
                    OfflineComponentType::Ledger => {
                        AnyWalletOffline::Ledger(LedgerKey::new(gargs.network)?)
                    }
                };
                let wallet = Wallet::new(wallet_name, offline_wallet, online_wallet)?;
                let wallet = Rc::new(RefCell::new(wallet));

                // Auto-feed
                if *auto_feed_xpubs
                    && !wallet.borrow().offline_wallet().is_none()
                    && !wallet.borrow().online_wallet().is_none()
                {
                    (WalletAXpubSubcmd::AutoAdd { count: 20 }).execute(Box::new(wallet.clone()))?;
                }
                wallet
            }
            _ => {
                let mut wallet = Wallet::load(&db, &wallet_name)?;
                if need_offline_component {
                    match wallet.offline_wallet_mut() {
                        AnyWalletOffline::None => (),
                        AnyWalletOffline::LocalKey(lk) => todo!(),
                        AnyWalletOffline::Ledger(ledger) => ledger.init_ledger_client()?,
                    };
                }
                if need_online_component {
                    match wallet.online_wallet_mut() {
                        AnyWalletOnline::None => (),
                        AnyWalletOnline::Service(sb) => sb.init_service_client(service_client)?,
                        AnyWalletOnline::Local(_) => todo!(),
                    };
                }
                Rc::new(RefCell::new(wallet))
            }
        };

        let res: Box<dyn crate::display::Displayable> = match self {
            WalletSubcmd::Create { .. } => {
                wallet.borrow().create(&mut db)?;
                Box::new("Wallet created")
            }
            WalletSubcmd::Remove => {
                wallet.borrow().delete(&mut db)?;
                Box::new("Wallet deleted")
            }
            WalletSubcmd::Descriptors { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::LedgerPolicies { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::HeritageConfigs { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::AccountXpubs { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::Sync => {
                wallet.borrow_mut().sync()?;
                Box::new("Synchronization done")
            }
            WalletSubcmd::Info => Box::new(wallet.borrow().get_wallet_info()?),
            WalletSubcmd::GetAddress => Box::new(wallet.borrow().get_address()?),
            WalletSubcmd::SendBitcoins {
                recipient,
                sign,
                broadcast,
                skip_confirmation,
            } => {
                let spending_config = NewTx::Recipients(
                    recipient
                        .into_iter()
                        .map(|(address, amount)| NewTxRecipient {
                            address: address.to_string(),
                            amount: amount.to_sat(),
                        })
                        .collect(),
                );
                // Get the PSBT
                let (mut psbt, summary) = wallet.borrow().create_psbt(spending_config)?;

                log::debug!("{}", serde_json::to_string_pretty(&summary)?);
                log::debug!("{}", serde_json::to_string_pretty(&psbt)?);
                if sign
                    && (skip_confirmation
                        || ask_user_confirmation(&format!(
                            "{}\nDo you want to sign this PSBT?",
                            serde_json::to_string_pretty(&psbt)?
                        ))?)
                {
                    log::info!("Signing the PSBT...");
                    // Sign it
                    let signed_input_count = wallet.borrow().sign_psbt(&mut psbt)?;
                    log::info!("Signed {signed_input_count} input(s)");
                    log::debug!("{}", serde_json::to_string_pretty(&psbt)?);

                    if broadcast
                        && (skip_confirmation
                            || ask_user_confirmation(&format!(
                                "{}\nDo you want to broadcast this PSBT?",
                                serde_json::to_string_pretty(&psbt)?
                            ))?)
                    {
                        log::info!("Broadcasting the PSBT...");
                        // Broadcast it
                        Box::new(wallet.borrow().broadcast(psbt)?)
                    } else {
                        Box::new(psbt.to_string())
                    }
                } else {
                    Box::new(psbt.to_string())
                }
            }
            WalletSubcmd::SignPsbt {
                mut psbt,
                broadcast,
                skip_confirmation,
            } => {
                log::debug!("{}", serde_json::to_string_pretty(&psbt)?);
                log::info!("Signing the PSBT...");
                // Sign it
                let signed_input_count = wallet.borrow().sign_psbt(&mut psbt)?;
                log::info!("Signed {signed_input_count} input(s)");
                log::debug!("{}", serde_json::to_string_pretty(&psbt)?);

                if broadcast
                    && (skip_confirmation
                        || ask_user_confirmation(&format!(
                            "{}\nDo you want to broadcast this PSBT?",
                            serde_json::to_string_pretty(&psbt)?
                        ))?)
                {
                    log::info!("Broadcasting the PSBT...");
                    // Broadcast it
                    Box::new(wallet.borrow().broadcast(psbt)?)
                } else {
                    Box::new(psbt.to_string())
                }
            }
            WalletSubcmd::BroadcastPsbt { psbt } => {
                log::debug!("{}", serde_json::to_string_pretty(&psbt)?);
                log::info!("Broadcasting the PSBT...");
                // Broadcast it
                Box::new(wallet.borrow().broadcast(psbt)?)
            }
            WalletSubcmd::DisplayPsbt { psbt } => todo!(),
        };
        wallet.borrow().save(&mut db)?;
        Ok(res)
    }
}

fn parse_recipient(val: &str) -> Result<(Address, Amount)> {
    if !val.contains(':') {
        return Err(Error::Generic(
            "invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned(),
        ));
    }

    let mut parts = val.split(':');
    let addr = parts.next().ok_or_else(|| {
        Error::Generic("invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned())
    })?;
    let addr = Address::from_str(addr)
        .map_err(|e| Error::Generic(e.to_string()))?
        .assume_checked();

    let amount = parts.next().ok_or_else(|| {
        Error::Generic("invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned())
    })?;
    let amount = amount
        .parse::<Amount>()
        .map_err(|e| Error::Generic(e.to_string()))?;

    if parts.next().is_some() {
        return Err(Error::Generic(
            "invalid recipient. Must be <ADDRESS>:<AMOUNT>".to_owned(),
        ));
    }

    Ok((addr, amount))
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OnlineComponentType {
    /// No online component, the resulting wallet will not be able to sync, generate addresses, etc... (it will be sign-only)
    None,
    /// Use the Heritage service as the online component
    Service,
    /// Use an Electrum server as the online component
    Electrum,
    /// Use a Bitcoin node as the online component
    Bitcoin,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OfflineComponentType {
    /// No offline component, the resulting wallet will not be able to sign transactions (it will be watch-only)
    None,
    /// Store the seed in the local database (discouraged unless you know what you do. Please use a password to protect the seed)
    Local,
    /// Use a Ledger hardware-wallet device
    Ledger,
}
