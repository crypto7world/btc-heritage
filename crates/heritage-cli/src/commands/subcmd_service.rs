use btc_heritage_wallet::{errors::Result, Database, HeritageServiceClient, Tokens};

/// Commands related purely to the Heritage service
#[derive(Debug, Clone, clap::Subcommand)]
pub enum ServiceSubcmd {
    /// Login to the Heritage service and store the resulting tokens in the internal database
    Login,
    /// Logout of the Heritage service and discard the previously stored tokens
    Logout,
    /// List the Heritage wallets already created in the Heritage service, if any
    ListWallets,
    /// List the Heirs declared in the Heritage service, if any
    ListHeirs,
    /// List the Heritages that you are - or will be - eligible to in Heritage service, if any
    ListHeritages,
}

impl super::CommandExecutor for ServiceSubcmd {
    fn execute(&self, cli_parser: &super::CliParser) -> Result<()> {
        let mut db = Database::new(&cli_parser.gargs.datadir, cli_parser.gargs.network)?;
        match self {
            ServiceSubcmd::Login => Tokens::new(
                &cli_parser.service_gargs.auth_url,
                &cli_parser.service_gargs.auth_client_id,
            )?
            .save(&mut db),
            ServiceSubcmd::Logout => todo!(),
            ServiceSubcmd::ListWallets => HeritageServiceClient::new(
                cli_parser.service_gargs.service_url.clone(),
                Tokens::load(&mut db)?,
            )
            .list_wallets(),
            ServiceSubcmd::ListHeirs => todo!(),
            ServiceSubcmd::ListHeritages => todo!(),
        }
    }
}
