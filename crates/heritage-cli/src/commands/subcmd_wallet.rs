use std::{any::Any, cell::RefCell, rc::Rc, str::FromStr};

use btc_heritage_wallet::{
    bitcoin::{bip32::Fingerprint, Address, Amount},
    AnyWalletOffline, AnyWalletOnline, Database, HeritageServiceClient, LedgerKey, ServiceBinding,
    Tokens, Wallet,
};
use clap::builder::{PossibleValuesParser, TypedValueParser};

use super::subcmd_wallet_axpubs::WalletAXpubSubcmd;

/// Sub-command for wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Creates a new Heritage wallet with the chosen online and offline components
    Create {
        #[arg(long, value_name = "TYPE", alias = "online", value_enum, default_value_t=OnlineComponentType::Service)]
        /// Specify the kind of online-component the wallet will use
        online_component: OnlineComponentType,
        #[arg(long, value_name = "NAME", conflicts_with_all=["existing_service_wallet_fingerprint", "existing_service_wallet_id"])]
        /// Specify the name of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        existing_service_wallet_name: Option<String>,
        #[arg(long, value_name = "FINGERPRINT", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_id"])]
        /// Specify the fingerprint of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        existing_service_wallet_fingerprint: Option<Fingerprint>,
        #[arg(long, value_name = "WALLET_ID", conflicts_with_all=["existing_service_wallet_name", "existing_service_wallet_fingerprint"])]
        /// Specify the ID of an existing Heritage wallet in the service
        /// to bind to, instead of creating a new one (if online_component = service)
        existing_service_wallet_id: Option<String>,
        #[arg(long, value_name = "TYPE", alias = "offline", value_enum, default_value_t=OfflineComponentType::Ledger, requires_if("local", "localgen"))]
        /// Specify the kind of offline-component the wallet will use
        offline_component: OfflineComponentType,
        #[arg(long, default_value_t = true)]
        /// Automaticly feed Heritage account eXtended public keys (xpubs) to the online component of the wallet if possible.
        auto_feed_xpubs: bool,
        #[arg(long, value_name = "WORD", num_args=2..24, group="localgen")]
        /// The mnemonic phrase to restore as a seed for the Heritage wallet (12, 18 or 24 words) (if offline_component = local).
        seed: Option<Vec<String>>,
        #[arg(
            long, value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()),
            group="localgen"
        )]
        /// The length of the mnemonic phrase to generate as a seed for the Heritage wallet (if offline_component = local).
        word_count: Option<usize>,
        #[arg(short = 'p', long, default_value_t = true)]
        /// Signal that the seed is password-protected (if offline_component = local).
        with_password: bool,
    },
    /// Remove the wallet from the database
    Remove,
    /// Manage the Heritage configuration of the wallet
    HeritageConfig,
    /// Commands managing the Account eXtended Public Keys of the wallet
    AccountXpubs {
        #[command(subcommand)]
        subcmd: super::subcmd_wallet_axpubs::WalletAXpubSubcmd,
    },
    /// Sync the wallet from the Bitcoin network
    Sync,
    /// Generate a new Bitcoin address for the Heritage wallet
    GetAddress,
    /// Create a Partially Signed Bitcoin Transaction (PSBT), a.k.a an Unsigned TX, from the provided receipients information
    SendBitcoins {
        #[arg(short, long, value_name="ADDRESS>:<AMOUNT", required = true, value_parser=parse_recipient)]
        /// A recipient for our BTC
        recipient: Vec<(Address, Amount)>,
        #[arg(short, long, default_value_t = false)]
        /// Immediately sign the PSBT
        sign: bool,
        #[arg(short, long, default_value_t = false, requires = "sign")]
        /// Immediately broadcast the PSBT after signing it
        broadcast: bool,
        #[arg(short = 'y', long, default_value_t = false, requires = "broadcast")]
        /// Broadcast without asking for confirmation{n}
        /// /!\ BE VERY CAREFULL with that option /!\.
        skip_confirmation: bool,
    },
    /// Sign every sign-able inputs of the given Partially Signed Bitcoin Transaction (PSBT)
    SignPsbt {
        /// The PSBT to sign
        psbt: String,
        #[arg(long, default_value_t = false)]
        /// Immediately broadcast the PSBT after signing it
        broadcast: bool,
    },
    /// Extract a raw transaction from the given Partially Signed Bitcoin Transaction (PSBT) and broadcast it to the Bitcoin network
    BroadcastPsbt {
        /// The PSBT to broadcast. Must have every inputs correctly signed for this to work.
        psbt: String,
    },
    /// Display infos on the given Partially Signed Bitcoin Transaction (PSBT)
    DisplayPsbt {
        /// The PSBT to display.
        psbt: String,
    },
}

impl super::CommandExecutor for WalletSubcmd {
    fn execute(
        self,
        params: Box<dyn Any>,
    ) -> btc_heritage_wallet::errors::Result<Box<dyn crate::display::Displayable>> {
        let (wallet_name, gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();
        let mut db = Database::new(&gargs.datadir, gargs.network)?;

        println!("wallet#test exist: {}", db.contains_key("wallet#test")?);
        let service_client =
            HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?);

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
                    (WalletAXpubSubcmd::AutoFeed { count: 20 })
                        .execute(Box::new(wallet.clone()))?;
                }
                wallet
            }
            _ => {
                let mut wallet = Wallet::load(&db, &wallet_name)?;
                match wallet.offline_wallet_mut() {
                    AnyWalletOffline::None => (),
                    AnyWalletOffline::LocalKey(lk) => todo!(),
                    AnyWalletOffline::Ledger(ledger) => ledger.init_ledger_client()?,
                };
                match wallet.online_wallet_mut() {
                    AnyWalletOnline::None => (),
                    AnyWalletOnline::Service(sb) => sb.init_service_client(service_client)?,
                    AnyWalletOnline::Local(_) => todo!(),
                };
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
            WalletSubcmd::HeritageConfig => todo!(),
            WalletSubcmd::AccountXpubs { subcmd } => subcmd.execute(Box::new(wallet.clone()))?,
            WalletSubcmd::Sync => todo!(),
            WalletSubcmd::GetAddress => todo!(),
            WalletSubcmd::SendBitcoins {
                recipient,
                sign,
                broadcast,
                skip_confirmation,
            } => todo!(),
            WalletSubcmd::SignPsbt { psbt, broadcast } => todo!(),
            WalletSubcmd::BroadcastPsbt { psbt } => todo!(),
            WalletSubcmd::DisplayPsbt { psbt } => todo!(),
        };
        wallet.borrow().save(&mut db)?;
        Ok(res)
    }
}

fn parse_recipient(val: &str) -> Result<(Address, Amount), String> {
    static ERR_MSG: &'static str = "invalid recipient. Must be <ADDRESS>:<AMOUNT>";

    if !val.contains(':') {
        return Err(ERR_MSG.to_string());
    }

    let mut parts = val.split(':');
    let addr = parts.next().ok_or_else(|| ERR_MSG.to_string())?;
    let addr = Address::from_str(addr)
        .map_err(|e| e.to_string())?
        .assume_checked();

    let amount = parts.next().ok_or_else(|| ERR_MSG.to_string())?;
    let amount = amount.parse::<Amount>().map_err(|e| e.to_string())?;

    if parts.next().is_some() {
        return Err(ERR_MSG.to_string());
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
