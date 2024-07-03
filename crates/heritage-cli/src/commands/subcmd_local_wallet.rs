/// Sub-commands for local wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum LocalWalletSubcmd {
    #[clap(flatten)]
    Common(super::subcmd_wallet::WalletSubcmd),
    /// Broadcast the given transaction to the Bitcoin network
    BroadcastTx,
}
