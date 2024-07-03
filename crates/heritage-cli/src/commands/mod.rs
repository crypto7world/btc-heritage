mod commands;
mod subcmd_service;
mod subcmd_wallet;

use btc_heritage_wallet::bitcoin::Network;

pub trait CommandExecutor {
    fn execute(&self, cli_parser: &CliParser) -> btc_heritage_wallet::errors::Result<()>;
}

#[derive(Clone, Debug, clap::Parser)]
/// The Heritage Wallet CLI
///
/// heritage-cli manages Heritage wallets with built-in inheritance and backup access.
/// It can work with the Heritage service or locally with a custom Bitcoin or Electrum node.
#[command(author= option_env ! ("CARGO_PKG_AUTHORS").unwrap_or(""), version = option_env ! ("CARGO_PKG_VERSION").unwrap_or("unknown"), about, long_about = None)]
pub struct CliParser {
    #[clap(next_help_heading = Some("Global options"))]
    #[command(flatten)]
    pub gargs: CliGlobalArgs,
    #[command(flatten)]
    pub service_gargs: ServiceGlobalArgs,
    #[command(flatten)]
    pub electrum_gargs: ElectrumGlobalArgs,
    #[command(flatten)]
    pub bitcoinrpc_gargs: BitcoinRpcGlobalArgs,
    #[command(subcommand)]
    /// Top level cli sub-commands.
    pub cmd: commands::Command,
}

impl CliParser {
    pub fn execute(&self) -> btc_heritage_wallet::errors::Result<()> {
        self.cmd.execute(self)
    }
}

#[derive(Clone, Debug, clap::Args)]
pub struct CliGlobalArgs {
    #[arg(
        short, long,
        default_value_t = Network::Bitcoin,
        global = true
    )]
    /// Set the Bitcoin network on which the CLI operates.
    pub network: Network,

    #[arg(
        short, long,
        value_hint = clap::ValueHint::DirPath,
        default_value = "~/.heritage-wallet",
        global = true
    )]
    /// Use the specified directory for database storage instead of the default one.
    pub datadir: String,
}

#[derive(Clone, Debug, clap::Args)]
pub struct ServiceGlobalArgs {
    #[arg(
        long,
        value_hint = clap::ValueHint::Url,
        default_value = "https://api.btcherit.com/v1",
        global = true
    )]
    /// Set the URL of the Heritage service API.
    pub service_url: String,
    #[arg(
        long,
        value_hint = clap::ValueHint::Url,
        default_value = "https://device.crypto7.world",
        global = true
    )]
    /// Set the URL of the Heritage service OAUTH authentication endpoint for the CLI.
    pub auth_url: String,
    #[arg(
        long,
        default_value = "cda6031ca00d09d66c2b632448eb8fef",
        global = true
    )]
    /// Set the OAUTH Client Id of the CLI for the Heritage service authentication endpoint.
    pub auth_client_id: String,
}

#[derive(Clone, Debug, clap::Args)]
pub struct ElectrumGlobalArgs {
    #[arg(
        long,
        value_hint = clap::ValueHint::Url,
        default_value = "http://localhost:50001",
        global = true
    )]
    /// Set the URL of an Electrum server RPC endpoint.
    pub electrum_url: String,
}

#[derive(Clone, Debug, clap::Args)]
pub struct BitcoinRpcGlobalArgs {
    #[arg(
        long,
        value_hint = clap::ValueHint::Url,
        default_value = "http://localhost:8332",
        global = true
    )]
    /// Set the URL of a Bitcoin node RPC endpoint.
    pub bitcoin_url: String,
}
