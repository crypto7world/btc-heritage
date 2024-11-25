use super::Blocker;

use crate::{errors::Result, types::*};

use btc_heritage::{bitcoin::psbt::Psbt, heritage_wallet::WalletAddress};

#[derive(Debug, Clone)]
pub struct HeritageServiceClient {
    inner: crate::async_client::HeritageServiceClient,
    blocker: &'static Blocker,
}

macro_rules! impl_blocking {
    ($fn_name:ident(& $self:ident $(,$a:ident : $t:ty)*) -> $ret:ty) => {
        pub fn $fn_name(& $self $(,$a : $t)*) -> $ret {
            $self.blocker.block_on($self.inner.$fn_name($($a),*))
        }
    };
}

impl HeritageServiceClient {
    pub fn new(service_api_url: String, tokens: Option<super::Tokens>) -> Self {
        Self {
            inner: crate::async_client::HeritageServiceClient::new(
                service_api_url,
                tokens.map(|t| t.inner),
            ),
            blocker: super::blocker(),
        }
    }

    ////////////////////////
    //      Wallets       //
    ////////////////////////
    impl_blocking!(list_wallets(&self) -> Result<Vec<HeritageWalletMeta>>);
    impl_blocking!(post_wallets(&self, create: HeritageWalletMetaCreate) -> Result<HeritageWalletMeta>);
    impl_blocking!(patch_wallet(&self, wallet_id: &str, name: Option<String>, block_inclusion_objective: Option<BlockInclusionObjective>) -> Result<HeritageWalletMeta>);
    impl_blocking!(get_wallet(&self, wallet_id: &str) -> Result<HeritageWalletMeta>);
    impl_blocking!(list_wallet_account_xpubs(&self, wallet_id: &str) -> Result<Vec<AccountXPubWithStatus>>);
    impl_blocking!(post_wallet_account_xpubs(&self, wallet_id: &str, account_xpubs: Vec<btc_heritage::AccountXPub>) -> Result<()>);
    impl_blocking!(list_wallet_heritage_configs(&self, wallet_id: &str) -> Result<Vec<HeritageConfig>>);
    impl_blocking!(post_wallet_heritage_configs(&self, wallet_id: &str, hc: HeritageConfig) -> Result<HeritageConfig>);
    impl_blocking!(list_wallet_transactions(&self, wallet_id: &str) -> Result<Vec<TransactionSummary>>);
    impl_blocking!(list_wallet_utxos(&self, wallet_id: &str) -> Result<Vec<HeritageUtxo>>);
    impl_blocking!(list_wallet_addresses(&self, wallet_id: &str) -> Result<Vec<WalletAddress>>);
    impl_blocking!(post_wallet_create_address(&self, wallet_id: &str) -> Result<String>);
    impl_blocking!(post_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization>);
    impl_blocking!(get_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization>);
    impl_blocking!(get_wallet_descriptors_backup(&self, wallet_id: &str) -> Result<HeritageWalletBackup>);
    impl_blocking!(post_wallet_create_unsigned_tx(&self, wallet_id: &str, new_tx: NewTx) -> Result<(Psbt, TransactionSummary)>);
    impl_blocking!(post_broadcast_tx(&self, psbt: Psbt) -> Result<Txid>);

    ////////////////////////
    //       Heirs        //
    ////////////////////////
    impl_blocking!(list_heirs(&self) -> Result<Vec<Heir>>);
    impl_blocking!(post_heirs(&self, heir_create: HeirCreate) -> Result<Heir>);
    impl_blocking!(get_heir(&self, heir_id: &str) -> Result<Heir>);
    impl_blocking!(patch_heir(&self, heir_id: &str, heir_update: HeirUpdate) -> Result<Heir>);
    impl_blocking!(post_heir_contacts(&self, heir_id: &str, contacts_to_add: Vec<HeirContact>) -> Result<()>);
    impl_blocking!(delete_heir_contacts(&self, heir_id: &str, contacts_to_delete: Vec<HeirContact>) -> Result<()>);

    ////////////////////////
    //     Heritages      //
    ////////////////////////
    impl_blocking!(list_heritages(&self) -> Result<Vec<Heritage>>);
    impl_blocking!(post_heritage_create_unsigned_tx(&self, heritage_id: &str, drain_to: NewTxDrainTo) -> Result<(Psbt, TransactionSummary)>);
}
