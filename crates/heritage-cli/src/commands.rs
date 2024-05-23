use clap::{Parser, Subcommand};

#[derive(PartialEq, Clone, Debug, Parser)]
/// The Heritage Wallet CLI
///
/// heritage-cli is a light weight command line allowing to manage Heritage wallets.
/// It can work with the Heritage service or with a custom Bitcoin or Electrum node.
#[command(author= option_env ! ("CARGO_PKG_AUTHORS").unwrap_or(""), version = option_env ! ("CARGO_PKG_VERSION").unwrap_or("unknown"), about, long_about = None)]
pub struct CliOpts {
    #[arg(short, long, value_hint = clap::ValueHint::DirPath, default_value = "~/.heritage-wallet")]
    /// Use the specified directory for storage instead of the default one.
    pub datadir: String,

    #[command(subcommand)]
    /// Top level cli sub-commands.
    pub subcommand: CliCommand,
}

/// Top level cli sub-commands.
#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum CliCommand {
    /// Login to the Heritage service, allowing interactions with it using the online-wallet commmands
    Login,
    /// Logout from the Heritage service
    Logout,
    #[command(subcommand)]
    /// All commands related to seed and private keys management
    Seed(SeedCommand),
    #[command(subcommand)]
    /// All commands related to wallets managed by the user with a custom Bitcoin or Electrum node
    OfflineWallet(OfflineWalletCommand),
    #[command(subcommand)]
    /// All commands related to wallets managed by the Heritage service
    OnlineWallet(OnlineWalletCommand),
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum SeedCommand {
    List,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum OfflineWalletCommand {
    List,
}

#[derive(Debug, Clone, Subcommand, PartialEq, Eq)]
pub enum OnlineWalletCommand {
    List,
}
