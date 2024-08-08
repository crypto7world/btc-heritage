use core::{any::Any, cell::RefCell};
use std::rc::Rc;

use btc_heritage_wallet::{
    btc_heritage::HeritageWalletBackup, errors::Result, Wallet, WalletOnline,
};

/// Wallet Descriptors management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletDescriptorsSubcmd {
    /// List all the Descriptors (BIP380) known by the Online component of the wallet
    Backup,
    /// Restore Descriptors (BIP380) in the Online component of the wallet. This will infer and
    /// create Heritage Configurations and used Account eXtended Public Keys. Can only be used on
    /// a wallet with a freshly created Online component (no Account XPub, no Heritage Configuration)
    Restore {
        /// The Descriptors to restore
        #[arg(short, long, required = true, value_parser=parse_descriptor_backup)]
        descriptors: HeritageWalletBackup,
    },
}

impl super::CommandExecutor for WalletDescriptorsSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletDescriptorsSubcmd::Backup => {
                Box::new(wallet.borrow().online_wallet().backup_descriptors()?)
            }
            WalletDescriptorsSubcmd::Restore { descriptors } => todo!(),
        };
        Ok(res)
    }
}

fn parse_descriptor_backup(val: &str) -> Result<HeritageWalletBackup> {
    Ok(serde_json::from_str(val)?)
}
