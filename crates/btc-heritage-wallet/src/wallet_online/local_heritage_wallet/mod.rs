use btc_heritage::bitcoin::bip32::Fingerprint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalHeritageWallet {
    heritage_wallet_id: String,
}

impl super::WalletOnline for LocalHeritageWallet {
    fn backup_descriptors(
        &self,
    ) -> crate::errors::Result<Vec<btc_heritage::heritage_wallet::DescriptorsBackup>> {
        todo!()
    }

    fn get_address(&self) -> crate::errors::Result<String> {
        todo!()
    }

    fn list_used_account_xpubs(&self) -> crate::errors::Result<Vec<btc_heritage::AccountXPub>> {
        todo!()
    }

    fn list_unused_account_xpubs(&self) -> crate::errors::Result<Vec<btc_heritage::AccountXPub>> {
        todo!()
    }

    fn feed_account_xpubs(
        &mut self,
        account_xpubs: &[btc_heritage::AccountXPub],
    ) -> crate::errors::Result<()> {
        todo!()
    }

    fn list_heritage_configs(&self) -> crate::errors::Result<Vec<btc_heritage::HeritageConfig>> {
        todo!()
    }

    fn set_heritage_config(
        &mut self,
        new_hc: &btc_heritage::HeritageConfig,
    ) -> crate::errors::Result<()> {
        todo!()
    }

    fn sync(&mut self) -> crate::errors::Result<()> {
        todo!()
    }

    fn get_balance(&self) -> crate::errors::Result<btc_heritage::HeritageWalletBalance> {
        todo!()
    }

    fn last_sync_ts(&self) -> crate::errors::Result<u64> {
        todo!()
    }

    fn create_psbt(
        &self,
        spending_config: btc_heritage::SpendingConfig,
    ) -> crate::errors::Result<(
        btc_heritage::PartiallySignedTransaction,
        btc_heritage::heritage_wallet::TransactionSummary,
    )> {
        todo!()
    }
}

impl crate::wallet::WalletCommons for LocalHeritageWallet {
    fn fingerprint(&self) -> crate::errors::Result<Option<Fingerprint>> {
        todo!()
    }

    fn network(&self) -> crate::errors::Result<btc_heritage::bitcoin::Network> {
        todo!()
    }
}
