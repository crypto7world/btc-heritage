use crate::{BoundFingerprint, Broadcaster};
use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Txid},
    heritage_wallet::WalletAddress,
    AccountXPub, HeritageConfig, HeritageWalletBackup, PartiallySignedTransaction,
};
use heritage_api_client::{AccountXPubWithStatus, NewTx, TransactionSummary};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalHeritageWallet {
    heritage_wallet_id: String,
}

impl super::OnlineWallet for LocalHeritageWallet {
    fn backup_descriptors(&self) -> crate::errors::Result<HeritageWalletBackup> {
        todo!()
    }

    fn get_address(&self) -> crate::errors::Result<String> {
        todo!()
    }

    fn list_addresses(&self) -> crate::errors::Result<Vec<WalletAddress>> {
        todo!()
    }

    fn list_transactions(&self) -> crate::errors::Result<Vec<TransactionSummary>> {
        todo!()
    }

    fn list_account_xpubs(&self) -> crate::errors::Result<Vec<AccountXPubWithStatus>> {
        todo!()
    }

    fn feed_account_xpubs(&mut self, account_xpubs: Vec<AccountXPub>) -> crate::errors::Result<()> {
        todo!()
    }

    fn list_heritage_configs(&self) -> crate::errors::Result<Vec<HeritageConfig>> {
        todo!()
    }

    fn set_heritage_config(
        &mut self,
        new_hc: HeritageConfig,
    ) -> crate::errors::Result<HeritageConfig> {
        todo!()
    }

    fn set_block_inclusion_objective(
        &mut self,
        bio: u16,
    ) -> crate::errors::Result<super::WalletStatus> {
        todo!()
    }

    fn sync(&mut self) -> crate::errors::Result<()> {
        todo!()
    }

    fn get_wallet_status(&self) -> crate::errors::Result<super::WalletStatus> {
        todo!()
    }

    fn create_psbt(
        &self,
        new_tx: NewTx,
    ) -> crate::errors::Result<(PartiallySignedTransaction, TransactionSummary)> {
        todo!()
    }
}

impl Broadcaster for LocalHeritageWallet {
    fn broadcast(&self, psbt: PartiallySignedTransaction) -> crate::errors::Result<Txid> {
        todo!()
    }
}
impl BoundFingerprint for LocalHeritageWallet {
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        todo!()
    }
}
