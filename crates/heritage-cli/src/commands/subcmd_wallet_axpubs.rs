use std::{any::Any, cell::RefCell, rc::Rc};

use btc_heritage_wallet::{
    errors::Result, AccountXPub, AccountXPubWithStatus, Wallet, WalletOffline, WalletOnline,
};

/// Wallet Account XPubs management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletAXpubSubcmd {
    /// List Account eXtended Public Keys generated using the Offline component of the wallet
    Generate {
        /// The number of Account eXtended Public Keys to generate
        #[arg(long, default_value_t = 20)]
        count: usize,
    },
    /// List the Account eXtended Public Keys already added by the Online component of the wallet and their status
    ListAdded {
        /// List the used Account eXtended Public Keys of the Online wallet
        #[arg(long, default_value_t = true)]
        used: bool,
        /// List the unused Account eXtended Public Keys of the Online wallet
        #[arg(long, default_value_t = true)]
        unused: bool,
    },
    /// Add Account eXtended Public Keys to the Online component of the wallet
    Add {
        /// The Account eXtended Public Key(s) to feed
        #[arg(value_name = "ACCOUNT_XPUB", num_args=1.., required = true, value_parser=parse_account_xpubs)]
        account_xpubs: Vec<AccountXPub>,
    },
    /// Generate Account eXtended Public Keys using the Offline component of the wallet and add them to the Online component
    AutoAdd {
        /// The number of Account eXtended Public Keys to add
        #[arg(long, default_value_t = 20)]
        count: usize,
    },
}

impl super::CommandExecutor for WalletAXpubSubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletAXpubSubcmd::ListAdded { used, unused } => {
                let mut res = wallet.borrow().list_account_xpubs()?;
                if !used {
                    res.retain(|e| {
                        match e {
                            AccountXPubWithStatus::Used(_) => false,
                            _ => true,
                        };
                        true
                    })
                }
                if !unused {
                    res.retain(|e| {
                        match e {
                            AccountXPubWithStatus::Unused(_) => false,
                            _ => true,
                        };
                        true
                    })
                }
                Box::new(res)
            }
            WalletAXpubSubcmd::Generate { count } => {
                Box::new(wallet.borrow().derive_accounts_xpubs(count)?)
            }
            WalletAXpubSubcmd::Add { account_xpubs } => {
                wallet.borrow_mut().feed_account_xpubs(account_xpubs)?;
                Box::new(())
            }
            WalletAXpubSubcmd::AutoAdd { count } => {
                let account_xpubs = wallet.borrow().derive_accounts_xpubs(count)?;
                wallet.borrow_mut().feed_account_xpubs(account_xpubs)?;
                Box::new(())
            }
        };
        Ok(res)
    }
}

fn parse_account_xpubs(val: &str) -> Result<AccountXPub> {
    Ok(AccountXPub::try_from(val)?)
}
