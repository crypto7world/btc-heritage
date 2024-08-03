use core::{any::Any, cell::RefCell};
use std::rc::Rc;

use btc_heritage_wallet::{errors::Result, Wallet, WalletOnline};

/// Wallet Addresses management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletAddressesSubcmd {
    /// Create a new Bitcoin address that can be used to receive Bitcoins
    New,
    /// List all the addresses created for the wallet
    List,
}

impl super::CommandExecutor for WalletAddressesSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletAddressesSubcmd::New => Box::new(wallet.borrow().online_wallet().get_address()?),
            WalletAddressesSubcmd::List => {
                Box::new(wallet.borrow().online_wallet().list_addresses()?)
            }
        };
        Ok(res)
    }
}
