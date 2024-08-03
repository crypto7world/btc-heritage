use core::any::Any;

use btc_heritage_wallet::{
    errors::Result,
    heritage_api_client::{HeritageServiceClient, Tokens},
    Database,
};

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
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (gargs, service_gargs): (super::CliGlobalArgs, super::ServiceGlobalArgs) =
            *params.downcast().unwrap();
        let mut db = Database::new(&gargs.datadir, gargs.network)?;
        let res: Box<dyn crate::display::Displayable> = match self {
            ServiceSubcmd::Login => {
                Tokens::new(&service_gargs.auth_url, &service_gargs.auth_client_id)?
                    .save(&mut db)
                    .map(Box::new)?
            }
            ServiceSubcmd::Logout => todo!(),
            ServiceSubcmd::ListWallets => {
                HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?)
                    .list_wallets()
                    .map(Box::new)?
            }
            ServiceSubcmd::ListHeirs => {
                HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?)
                    .list_heirs()
                    .map(Box::new)?
            }
            ServiceSubcmd::ListHeritages => {
                HeritageServiceClient::new(service_gargs.service_api_url, Tokens::load(&mut db)?)
                    .list_heritages()
                    .map(Box::new)?
            }
        };
        Ok(res)
    }
}
