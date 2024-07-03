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
    fn execute(&self, cli_parser: &super::CliParser) -> btc_heritage_wallet::errors::Result<()> {
        match self {
            Command::Service { subcmd } => subcmd.execute(cli_parser),
            Command::Wallet {
                wallet_name: _,
                subcmd,
            } => subcmd.execute(cli_parser),
        }
    }
}
