use core::any::Any;

use btc_heritage_wallet::{
    bitcoin::psbt::Psbt, Database, DatabaseItem, Heir, HeirWallet, PsbtSummary, Wallet,
};

use crate::utils::get_fingerprints;

/// Top level cli sub-commands.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Commands related purely to the Heritage service
    Service {
        #[command(subcommand)]
        subcmd: super::subcmd_service::ServiceSubcmd,
    },
    /// Commands managing an Heritage wallet
    Wallet {
        /// The name of the wallet to operate
        ///
        /// Defaults to "default" or any other name you set with the "default-wallet-name" command
        wallet_name: Option<String>,
        #[command(subcommand)]
        subcmd: super::subcmd_wallet::WalletSubcmd,
    },
    /// List the existing Heritage wallet names, if any
    ListWallets,
    /// Display or set the default wallet name
    DefaultWalletName {
        /// Set the default wallet name instead of simply displaying it
        #[arg(short = 's', long = "set")]
        new_name: Option<String>,
    },
    /// Display infos on the given Partially Signed Bitcoin Transaction (PSBT)
    DisplayPsbt {
        /// The PSBT
        psbt: Psbt,
    },
    /// Commands managing heirs declared to ease the creation of wallet Heritage Configurations
    Heir {
        /// The name of the heir to operate
        ///
        /// Defaults to "default" or any other name you set with the "default-heir-name" command
        heir_name: Option<String>,
        #[command(subcommand)]
        subcmd: super::subcmd_heir::HeirSubcmd,
    },
    /// List the existing Heirs, if any
    ListHeirs,
    /// Display or set the default heir name
    DefaultHeirName {
        /// Set the default heir name instead of simply displaying it
        #[arg(short = 's', long = "set")]
        new_name: Option<String>,
    },
    /// Commands managing heir-wallets, restricted wallets used solely to claim inheritances
    HeirWallet {
        /// The name of the heir-wallet to operate
        ///
        /// Defaults to "default" or any other name you set with the "default-heir-wallet-name" command
        heir_wallet_name: Option<String>,
        #[command(subcommand)]
        subcmd: super::subcmd_heir::HeirSubcmd,
    },
    /// List the existing heir-wallets, if any
    ListHeirWallets,
    /// Display or set the default heir-wallet name
    DefaultHeirWalletName {
        /// Set the default heir-wallet name instead of simply displaying it
        #[arg(short = 's', long = "set")]
        new_name: Option<String>,
    },
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
        let mut db = Database::new(&gargs.datadir, gargs.network)?;
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
            Command::ListWallets => {
                let wallet_names = Wallet::list_names(&db)?;
                Ok(Box::new(wallet_names))
            }
            Command::DefaultWalletName { new_name } => {
                if let Some(new_name) = new_name {
                    Wallet::set_default_item_name(&mut db, new_name)?;
                }
                Ok(Box::new(Wallet::get_default_item_name(&db)?))
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
                let params = Box::new((db, heir_name, gargs, service_gargs));
                subcmd.execute(params)
            }
            Command::ListHeirs => {
                let heir_names = Heir::list_names(&db)?;
                Ok(Box::new(heir_names))
            }
            Command::DefaultHeirName { new_name } => {
                if let Some(new_name) = new_name {
                    Heir::set_default_item_name(&mut db, new_name)?;
                }
                Ok(Box::new(Heir::get_default_item_name(&db)?))
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
            Command::ListHeirWallets => {
                let heir_wallet_names = HeirWallet::list_names(&db)?;
                Ok(Box::new(heir_wallet_names))
            }
            Command::DefaultHeirWalletName { new_name } => {
                if let Some(new_name) = new_name {
                    HeirWallet::set_default_item_name(&mut db, new_name)?;
                }
                Ok(Box::new(HeirWallet::get_default_item_name(&db)?))
            }
        }
    }
}
