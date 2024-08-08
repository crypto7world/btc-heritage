use core::{any::Any, cell::RefCell};
use std::rc::Rc;

use btc_heritage_wallet::{
    errors::Result, Database, DatabaseItem, LedgerPolicy, Wallet, WalletOnline,
};

/// Wallet Ledger Policy management subcommand.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletLedgerPolicySubcmd {
    /// List the Ledger policies (Bitcoin descriptors in a Ledger format) of the wallet
    List,
    /// List the Ledger policies of the wallet that are already registered in the Ledger
    ListRegistered,
    /// Register policies on a Ledger device
    Register {
        /// The policies to register.
        #[arg(value_name = "POLICY", num_args=1.., value_parser=parse_descriptor_backup)]
        policies: Vec<LedgerPolicy>,
    },
    /// Retrieve Ledger policies using the Online component of the wallet and register them to the Offline component
    AutoRegister,
}

impl super::CommandExecutor for WalletLedgerPolicySubcmd {
    fn execute(self, params: Box<dyn Any>) -> Result<Box<dyn crate::display::Displayable>> {
        let (wallet, mut db): (Rc<RefCell<Wallet>>, Database) = *params.downcast().unwrap();
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
                let btc_heritage_wallet::AnyKeyProvider::Ledger(ledger_wallet) =
                    wallet_ref.key_provider()
                else {
                    return Err(btc_heritage_wallet::errors::Error::IncorrectKeyProvider(
                        "Ledger",
                    ));
                };
                Box::new(ledger_wallet.list_registered_policies())
            }
            WalletLedgerPolicySubcmd::Register { policies } => {
                let mut wallet_ref_mut = wallet.borrow_mut();
                let btc_heritage_wallet::AnyKeyProvider::Ledger(ledger_wallet) =
                    wallet_ref_mut.key_provider_mut()
                else {
                    return Err(btc_heritage_wallet::errors::Error::IncorrectKeyProvider(
                        "Ledger",
                    ));
                };
                let count = ledger_wallet.register_policies(&policies)?;
                wallet.borrow().save(&mut db)?;
                Box::new(format!("{count} policies registered"))
            }
            WalletLedgerPolicySubcmd::AutoRegister => {
                let policies = wallet
                    .borrow()
                    .online_wallet()
                    .backup_descriptors()?
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, d)| {
                        TryInto::<LedgerPolicy>::try_into(d)
                            .map_err(|e| {
                                log::warn!(
                                    "Cannot convert Descriptor Backup #{i} into a LedgerPolicy: {e}"
                                );
                                e
                            })
                            .ok()
                    })
                    .collect::<Vec<_>>();
                let mut wallet_ref_mut = wallet.borrow_mut();
                let btc_heritage_wallet::AnyKeyProvider::Ledger(ledger_wallet) =
                    wallet_ref_mut.key_provider_mut()
                else {
                    return Err(btc_heritage_wallet::errors::Error::IncorrectKeyProvider(
                        "Ledger",
                    ));
                };
                let count = ledger_wallet.register_policies(&policies)?;
                wallet.borrow().save(&mut db)?;
                Box::new(format!("{count} new policies registered"))
            }
        };
        Ok(res)
    }
}

fn parse_descriptor_backup(val: &str) -> Result<LedgerPolicy> {
    Ok(val.try_into()?)
}
