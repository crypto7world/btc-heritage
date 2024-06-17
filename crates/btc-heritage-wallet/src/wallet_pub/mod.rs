use crate::errors::Result;
use btc_heritage::{
    bitcoin::bip32::Fingerprint,
    heritage_wallet::{DescriptorsBackup, TransactionSummary},
    PartiallySignedTransaction, SpendingConfig,
};

mod local_heritage_wallet;
mod service_client;
use local_heritage_wallet::LocalHeritageWallet;
use service_client::HeritageServiceClient;

/// This trait regroup the functions of an Heritage wallet that does not need
/// access to the private keys and can be safely operated in an online environment.
pub trait HeritageWalletPub {
    fn backup_descriptors(&self) -> Result<Vec<DescriptorsBackup>>;
    fn get_address(&self) -> Result<String>;
    fn sync(&self) -> Result<()>;
    fn create_psbt(
        &self,
        spending_config: SpendingConfig,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)>;
    /// Return the [Fingerprint] associated with the [AccountXPub](btc_heritage::AccountXPub) of the underlying [HeritageWallet](btc_heritage::HeritageWallet)
    fn fingerprint(&self) -> Result<Fingerprint>;
}

pub enum AnyHeritageWalletPub {
    None,
    Service(HeritageServiceClient),
    Local(LocalHeritageWallet),
}
