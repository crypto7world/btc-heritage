/// Sub-commands for local wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum ServiceWalletSubcmd {
    /// Login to the Heritage service, allowing interactions with it using the service-wallet commmands
    Login,
    /// Logout from the Heritage service
    Logout,
    /// Bind an existing Heritage wallet on the service with a seed or an hardware wallet
    Bind(super::subcmd_wallet_commons::KeySourceRestoreArgs),
    #[clap(flatten)]
    Common(super::subcmd_wallet_commons::WalletSubcmd),
}

/// Common options for service related commands.
#[derive(Debug, Clone, clap::Args)]
pub struct ServiceArgs {
    #[arg(
        long,
        default_value = "api.heritage.crypto7.world",
        global = true,
        display_order = 100
    )]
    /// The URL of the Heritage service.
    pub service_url: String,
}
