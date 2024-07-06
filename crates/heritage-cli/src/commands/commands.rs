use std::any::Any;

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
        wallet_name: String,
        #[command(subcommand)]
        subcmd: super::subcmd_wallet::WalletSubcmd,
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
        match self {
            Command::Service { subcmd } => {
                let params = Box::new((gargs, service_gargs));
                subcmd.execute(params)
            }
            Command::Wallet {
                wallet_name,
                subcmd,
            } => {
                let params = Box::new((
                    wallet_name,
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
