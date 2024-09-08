use core::any::Any;

use btc_heritage_wallet::{
    btc_heritage::BlockInclusionObjective, errors::Result,
    heritage_service_api_client::HeritageServiceClient,
};

/// Sub-command for service wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Get infos on the wallet
    Get,
    /// Update the wallet in the service
    Update {
        /// Change the name of the wallet
        #[arg(long)]
        name: Option<String>,
        /// Change the Block Inclusion Objective of the wallet. It is used to compute the fee when creating a new transaction
        #[arg(long, visible_alias = "bio")]
        block_inclusion_objective: Option<u16>,
    },
}

impl super::CommandExecutor for WalletSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (wallet_id, service_client): (String, HeritageServiceClient) =
            *params.downcast().unwrap();

        let res: Box<dyn crate::display::Displayable> = match self {
            WalletSubcmd::Get => Box::new(service_client.get_wallet(&wallet_id)?),
            WalletSubcmd::Update {
                name,
                block_inclusion_objective,
            } => {
                let block_inclusion_objective =
                    block_inclusion_objective.map(BlockInclusionObjective::from);
                Box::new(service_client.patch_wallet(
                    &wallet_id,
                    name,
                    block_inclusion_objective,
                )?)
            }
        };
        Ok(res)
    }
}
