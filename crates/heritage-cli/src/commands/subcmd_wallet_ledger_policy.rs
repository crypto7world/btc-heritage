use std::{any::Any, cell::RefCell, rc::Rc};

use btc_heritage_wallet::{errors::Result, LedgerPolicy, Wallet, WalletOnline};

/// Wallet Ledger Policy management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletLedgerPolicySubcmd {
    /// List the Ledger policies (Bitcoin descriptors in a Ledger format) of the wallet
    List,
    /// List the Ledger policies of the wallet that are already registered in the Ledger
    ListRegistered,
    /// Register policies on a Ledger device
    Register {
        #[arg(short, long, value_parser=parse_descriptor_backup)]
        /// The policies to register. If none is provided, the CLI will attempt
        /// to find them using the Online component of the wallet
        policies: Vec<LedgerPolicy>,
    },
}

impl super::CommandExecutor for WalletLedgerPolicySubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let wallet: Rc<RefCell<Wallet>> = *params.downcast().unwrap();
        let wallet = wallet.as_ref();
        let res: Box<dyn crate::display::Displayable> = match self {
            WalletLedgerPolicySubcmd::List => Box::new(
                wallet
                    .borrow()
                    .online_wallet()
                    .backup_descriptors()?
                    .into_iter()
                    .filter_map(|d| TryInto::<LedgerPolicy>::try_into(d).ok())
                    .collect::<Vec<_>>(),
            ),
            WalletLedgerPolicySubcmd::ListRegistered => {
                let wallet_ref = wallet.borrow();
                let btc_heritage_wallet::AnyWalletOffline::Ledger(ledger_wallet) =
                    wallet_ref.offline_wallet()
                else {
                    return Err(
                        btc_heritage_wallet::errors::Error::IncorrectOfflineComponent("Ledger"),
                    );
                };
                Box::new(ledger_wallet.list_registered_policies())
            }
            WalletLedgerPolicySubcmd::Register { policies } => {
                let mut wallet_ref_mut = wallet.borrow_mut();
                let btc_heritage_wallet::AnyWalletOffline::Ledger(ledger_wallet) =
                    wallet_ref_mut.offline_wallet_mut()
                else {
                    return Err(
                        btc_heritage_wallet::errors::Error::IncorrectOfflineComponent("Ledger"),
                    );
                };
                let policies = if policies.len() > 0 {
                    policies
                } else {
                    wallet
                        .borrow()
                        .online_wallet()
                        .backup_descriptors()?
                        .into_iter()
                        .filter_map(|d| TryInto::<LedgerPolicy>::try_into(d).ok())
                        .collect::<Vec<_>>()
                };
                let count = ledger_wallet.register_policies(&policies)?;
                Box::new(format!("{count} policies registered"))
            }
        };
        Ok(res)
    }
}

fn parse_descriptor_backup(val: &str) -> Result<LedgerPolicy> {
    Ok(val.try_into()?)
}
