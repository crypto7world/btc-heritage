use std::{any::Any, cell::RefCell, rc::Rc};

use btc_heritage_wallet::{errors::Result, HeritageConfig, Wallet, WalletOnline};

/// Wallet Heritage Configuration management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletHeritageConfigSubcmd {
    /// List the Heritage Configurations known by the Online component of the wallet
    List,
    /// Display the current Heritage Configuration of the Online component of the wallet
    ShowCurrent,
    /// Set a new Heritage Conguration for the Online component of the wallet
    Set {
        /// The Heritage Configuration as a JSON (tmp)
        #[arg(value_parser=parse_heritage_configuration)]
        heritage_config: HeritageConfig,
    },
}

impl super::CommandExecutor for WalletHeritageConfigSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletHeritageConfigSubcmd::List => Box::new(wallet.borrow().list_heritage_configs()?),
            WalletHeritageConfigSubcmd::ShowCurrent => {
                Box::new(wallet.borrow().list_heritage_configs()?.remove(0))
            }
            WalletHeritageConfigSubcmd::Set { heritage_config } => todo!(),
        };
        Ok(res)
    }
}

fn parse_heritage_configuration(val: &str) -> Result<HeritageConfig> {
    Ok(serde_json::from_str(val)?)
}
