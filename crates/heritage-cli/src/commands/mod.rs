mod commands;
mod subcmd_local_wallet;
mod subcmd_service_wallet;
mod subcmd_wallet_commons;

use btc_heritage_wallet::bitcoin::Network;

#[derive(Clone, Debug, clap::Parser)]
/// The Heritage Wallet CLI
///
/// heritage-cli manages Heritage wallets with built-in inheritance and backup access.
/// It can work with the Heritage service or locally with a custom Bitcoin or Electrum node.
#[command(author= option_env ! ("CARGO_PKG_AUTHORS").unwrap_or(""), version = option_env ! ("CARGO_PKG_VERSION").unwrap_or("unknown"), about, long_about = None)]
pub struct CliParser {
    #[command(flatten)]
    /// Use the specified directory for database storage instead of the default one.
    pub args: CliArgs,

    #[command(subcommand)]
    /// Top level cli sub-commands.
    pub cmd: commands::Command,
}

#[derive(Clone, Debug, clap::Args)]
pub struct CliArgs {
    #[arg(short, long, default_value_t = Network::Bitcoin)]
    /// Set the Bitcoin network on which the CLI operates.
    pub network: Network,

    #[arg(short, long, value_hint = clap::ValueHint::DirPath, default_value = "~/.heritage-wallet")]
    /// Use the specified directory for database storage instead of the default one.
    pub datadir: String,
}
