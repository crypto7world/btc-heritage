use core::any::Any;

use btc_heritage_wallet::{
    bitcoin::psbt::Psbt, Database, DatabaseItem, Heir, HeirWallet, PsbtSummary, Wallet,
};

use crate::utils::get_fingerprints;

use super::CommandExecutor;

/// Top level cli sub-commands.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Commands to login in the Heritage service and manage wallets, heirs and heritages in the service.
    /// {n}Usually only useful for login
    Service {
        #[command(subcommand)]
        subcmd: super::subcmd_service::ServiceSubcmd,
    },
    /// Commands managing wallet, use this to create and manage Heritage wallets.
    #[command(visible_aliases = ["wallets", "w"])]
    Wallet {
        /// The name of the wallet to operate.
        /// Defaults to "default" or any other name you set with the "default-name" command
        wallet_name: Option<String>,
        #[command(subcommand)]
        subcmd: ListAndDefault<super::subcmd_wallet::WalletSubcmd, Wallet>,
    },
    /// Commands managing heirs, use this to create or declare heirs for your Heritage wallet
    /// {n}While very convenient, declaring heirs explicitly is not necessary.
    #[command(visible_aliases = ["heirs", "h"])]
    Heir {
        /// The name of the heir to operate.
        /// Defaults to "default" or any other name you set with the "default-name" command
        heir_name: Option<String>,
        #[command(subcommand)]
        subcmd: ListAndDefault<super::subcmd_heir::HeirSubcmd, Heir>,
    },

    /// Commands managing heir-wallets, restricted wallets used only to list and claim inheritances
    /// {n}Use this if you are an heir and just want to claim an inheritance.
    #[command(visible_aliases = ["heir-wallets", "hw"])]
    HeirWallet {
        /// The name of the heir-wallet to operate.
        /// Defaults to "default" or any other name you set with the "default-name" command
        heir_wallet_name: Option<String>,
        #[command(subcommand)]
        subcmd: ListAndDefault<super::subcmd_heirwallet::HeirWalletSubcmd, HeirWallet>,
    },

    /// Display infos on the given Partially Signed Bitcoin Transaction (PSBT)
    DisplayPsbt {
        /// The PSBT
        psbt: Psbt,
    },
}

#[derive(Debug, clap::Subcommand)]
pub enum ListAndDefault<
    T: Clone + core::fmt::Debug + clap::Subcommand + CommandExecutor,
    I: DatabaseItem,
> {
    /// List all items in the database
    List,
    /// Display or set the default name
    DefaultName {
        /// Set the default name instead of simply displaying it
        #[arg(short = 's', long = "set")]
        new_name: Option<String>,
    },
    #[command(flatten)]
    Others(T),
    #[command(skip)]
    _Impossible {
        _i: core::convert::Infallible,
        _p: core::marker::PhantomData<I>,
    },
}
impl<T: Clone + core::fmt::Debug + clap::Subcommand + CommandExecutor, I: DatabaseItem> Clone
    for ListAndDefault<T, I>
{
    fn clone(&self) -> Self {
        match self {
            Self::List => Self::List,
            Self::DefaultName { new_name } => Self::DefaultName {
                new_name: new_name.clone(),
            },
            Self::Others(arg0) => Self::Others(arg0.clone()),
            Self::_Impossible { .. } => unreachable!(),
        }
    }
}

impl<T: Clone + core::fmt::Debug + clap::Subcommand + CommandExecutor, I: DatabaseItem>
    super::CommandExecutor for ListAndDefault<T, I>
{
    fn execute(
        self,
        params: Box<dyn Any>,
    ) -> btc_heritage_wallet::errors::Result<Box<dyn crate::display::Displayable>> {
        let (mut db, name, gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            Database,
            String,
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();

        match self {
            ListAndDefault::List => {
                let wallet_names = I::list_names(&db)?;
                Ok(Box::new(wallet_names))
            }
            ListAndDefault::DefaultName { new_name } => {
                if let Some(new_name) = new_name {
                    I::set_default_item_name(&mut db, new_name)?;
                }
                Ok(Box::new(I::get_default_item_name(&db)?))
            }
            ListAndDefault::Others(sub) => {
                let params = Box::new((
                    db,
                    name,
                    gargs,
                    service_gargs,
                    electrum_gargs,
                    bitcoinrpc_gargs,
                ));
                sub.execute(params)
            }
            ListAndDefault::_Impossible { .. } => unreachable!(),
        }
    }
}

impl super::CommandExecutor for Command {
    fn execute(
        self,
        params: Box<dyn Any>,
    ) -> btc_heritage_wallet::errors::Result<Box<dyn crate::display::Displayable>> {
        let (gargs, service_gargs, electrum_gargs, bitcoinrpc_gargs): (
            super::CliGlobalArgs,
            super::ServiceGlobalArgs,
            super::ElectrumGlobalArgs,
            super::BitcoinRpcGlobalArgs,
        ) = *params.downcast().unwrap();
        let db = Database::new(&gargs.datadir, gargs.network)?;
        match self {
            Command::Service { subcmd } => {
                let params = Box::new((db, service_gargs));
                subcmd.execute(params)
            }
            Command::Wallet {
                wallet_name,
                subcmd,
            } => {
                let wallet_name = match wallet_name {
                    Some(wn) => wn,
                    None => Wallet::get_default_item_name(&db)?,
                };
                let params = Box::new((
                    db,
                    wallet_name,
                    gargs,
                    service_gargs,
                    electrum_gargs,
                    bitcoinrpc_gargs,
                ));
                subcmd.execute(params)
            }
            Command::DisplayPsbt { psbt } => {
                let network = gargs.network;
                let summary = PsbtSummary::try_from((&psbt, &get_fingerprints(&db)?, network))?;
                Ok(Box::new(summary))
            }
            Command::Heir { heir_name, subcmd } => {
                let heir_name = match heir_name {
                    Some(wn) => wn,
                    None => Heir::get_default_item_name(&db)?,
                };
                let params = Box::new((
                    db,
                    heir_name,
                    gargs,
                    service_gargs,
                    electrum_gargs,
                    bitcoinrpc_gargs,
                ));
                subcmd.execute(params)
            }
            Command::HeirWallet {
                heir_wallet_name,
                subcmd,
            } => {
                let heir_wallet_name = match heir_wallet_name {
                    Some(wn) => wn,
                    None => HeirWallet::get_default_item_name(&db)?,
                };
                let params = Box::new((
                    db,
                    heir_wallet_name,
                    gargs,
                    service_gargs,
                    electrum_gargs,
                    bitcoinrpc_gargs,
                ));
                subcmd.execute(params)
            }
        }
    }
}
