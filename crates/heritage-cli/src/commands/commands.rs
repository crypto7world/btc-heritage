/// Top level cli sub-commands.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum Command {
    /// Lists the Heritage wallets present in the database
    ListWallets,
    /// Manage a local Heritage wallet, with a custom Bitcoin or Electrum node
    LocalWallet {
        #[command(flatten)]
        backend: BackendArg,
        /// The name of the wallet to operate
        wallet_name: String,
        #[command(subcommand)]
        subcmd: super::subcmd_local_wallet::LocalWalletSubcmd,
    },
    /// Manage a service Heritage wallet on the Heritage service
    ServiceWallet {
        #[command(flatten)]
        service_opts: super::subcmd_service_wallet::ServiceArgs,
        /// The name of the wallet to operate
        wallet_name: String,
        #[command(subcommand)]
        subcmd: super::subcmd_service_wallet::ServiceWalletSubcmd,
    },
}

/// Common options for local node related commands.
#[derive(Debug, Clone, clap::Args)]

pub struct BackendArg {
    #[arg(
        long,
        default_value = "localhost:8832",
        global = true,
        conflicts_with = "electrum_url",
        display_order = 100
    )]
    /// The URL of the RPC endpoint of a Bitcoin node.
    pub bitcoin_url: String,
    #[arg(
        long,
        global = true,
        conflicts_with = "bitcoin_url",
        display_order = 100
    )]
    /// The URL of the RPC endpoint of an Electrum node.
    pub electrum_url: Option<String>,
}
