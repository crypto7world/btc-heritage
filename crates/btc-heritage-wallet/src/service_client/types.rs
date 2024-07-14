use btc_heritage::{
    bitcoin::{bip32::Fingerprint, FeeRate},
    AccountXPub, BlockInclusionObjective, HeritageWalletBalance,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeritageWalletMeta {
    #[serde(rename = "wallet_id")]
    pub id: String,
    pub fingerprint: Option<Fingerprint>,
    pub last_sync_ts: u64,
    pub name: String,
    #[serde(default)]
    pub balance: Option<HeritageWalletBalance>,
    #[serde(default)]
    pub block_inclusion_objective: Option<BlockInclusionObjective>,
    #[serde(default)]
    pub fee_rate: Option<FeeRate>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "accountxpub",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum AccountXPubWithStatus {
    Used(AccountXPub),
    Unused(AccountXPub),
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NewTxRecipient {
    pub address: String,
    pub amount: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct NewTxDrainTo {
    pub drain_to: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NewTx {
    Recipients(Vec<NewTxRecipient>),
    DrainTo(NewTxDrainTo),
}
