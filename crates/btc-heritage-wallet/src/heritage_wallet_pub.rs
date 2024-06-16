use crate::errors::Result;
use btc_heritage::{
    database::memory::HeritageMemoryDatabase, heritage_wallet::TransactionSummary, HeritageWallet,
    PartiallySignedTransaction, SpendingConfig,
};

use crate::heritage_service_client::HeritageServiceClient;

/// This trait regroup the functions of an Heritage wallet that does not need
/// access to the private keys and can be safely operated in an online environment.
trait HeritageWalletPub {
    fn get_address(&self) -> Result<String>;
    fn sync(&self) -> Result<()>;
    fn create_psbt(
        &self,
        spending_config: SpendingConfig,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)>;
}

pub enum AnyHeritageWalletPub {
    None,
    Service(HeritageServiceClient),
    Local(HeritageWallet<HeritageMemoryDatabase>),
}
