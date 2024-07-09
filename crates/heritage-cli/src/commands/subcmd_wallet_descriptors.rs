use std::{any::Any, cell::RefCell, rc::Rc};

use btc_heritage_wallet::{errors::Result, DescriptorsBackup, Wallet};

/// Wallet Descriptors management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletDescriptorsSubcmd {
    /// List all the Descriptors (BIP380) known by the Online component of the wallet
    Backup,
    /// Restore Descriptors (BIP380) in the Online component of the wallet. This will infer and
    /// create Heritage Configurations and used Account eXtended Public Keys. Can only be used on
    /// a wallet with a freshly created Online component (no Account XPub, no Heritage Configuration)
    Restore {
        #[arg(short, long, required = true, value_parser=parse_descriptor_backup)]
        /// The Descriptors to restore
        descriptors: Vec<DescriptorsBackup>,
    },
    /// Register the Descriptors in a Ledger device as wallet policies
    RegisterOnLedger {
        #[arg(short, long, value_parser=parse_descriptor_backup)]
        /// The Descriptors to register. If none is provided, the CLI will attempt
        /// to use the current backup of the Online component of the wallet
        descriptors: Vec<DescriptorsBackup>,
    },
}

impl super::CommandExecutor for WalletDescriptorsSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletDescriptorsSubcmd::Backup => todo!(),
            WalletDescriptorsSubcmd::Restore { descriptors } => todo!(),
            WalletDescriptorsSubcmd::RegisterOnLedger { descriptors } => todo!(),
        };
        Ok(res)
    }
}

fn parse_descriptor_backup(val: &str) -> Result<DescriptorsBackup> {
    Ok(serde_json::from_str(val)?)
}
