use std::{any::Any, borrow::Borrow, cell::RefCell, rc::Rc};

use btc_heritage_wallet::{AccountXPub, Wallet, WalletOffline, WalletOnline};

/// Wallet Account XPubs management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletAXpubSubcmd {
    /// List the Account eXtended Public Keys known by the Online component of the wallet and their status
    List {
        #[arg(long, default_value_t = true)]
        /// List the used Account eXtended Public Keys of the Online wallet
        used: bool,
        #[arg(long, default_value_t = true)]
        /// List the unused Account eXtended Public Keys of the Online wallet
        unused: bool,
    },
    /// Display Account eXtended Public Keys generated using the Offline component of the wallet
    Display {
        #[arg(long, default_value_t = 20)]
        /// The number of Account eXtended Public Keys to generate
        count: usize,
    },
    /// Feed (add) Account eXtended Public Keys to the Online component of the wallet
    Feed {
        #[arg(value_name = "ACCOUNT_XPUB", num_args=1.., required = true, value_parser=parse_account_xpubs)]
        /// The Account eXtended Public Key(s) to feed
        account_xpubs: Vec<AccountXPub>,
    },
    /// Generate Account eXtended Public Keys using the Offline component of the wallet and feed them to the Online component
    AutoFeed {
        #[arg(long, default_value_t = 20)]
        /// The number of Account eXtended Public Keys to generate
        count: usize,
    },
}

impl super::CommandExecutor for WalletAXpubSubcmd {
    fn execute(
        self,
        params: Box<dyn Any>,
    ) -> btc_heritage_wallet::errors::Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletAXpubSubcmd::List { used, unused } => {
                let mut res = vec![];
                if used {
                    res.extend(wallet.borrow().list_used_account_xpubs()?.into_iter());
                }
                if unused {
                    res.extend(wallet.borrow().list_unused_account_xpubs()?.into_iter());
                }
                Box::new(res)
            }
            WalletAXpubSubcmd::Display { count } => {
                Box::new(wallet.borrow().derive_accounts_xpubs(count)?)
            }
            WalletAXpubSubcmd::Feed { account_xpubs } => {
                wallet.borrow_mut().feed_account_xpubs(&account_xpubs)?;
                Box::new(())
            }
            WalletAXpubSubcmd::AutoFeed { count } => {
                let account_xpubs = wallet.borrow().derive_accounts_xpubs(count)?;
                wallet.borrow_mut().feed_account_xpubs(&account_xpubs)?;
                Box::new(())
            }
        };
        Ok(res)
    }
}

fn parse_account_xpubs(val: &str) -> Result<AccountXPub, String> {
    AccountXPub::try_from(val).map_err(|e| e.to_string())
}
