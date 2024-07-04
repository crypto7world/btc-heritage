use btc_heritage::{bitcoin::FeeRate, BlockInclusionObjective, HeritageWalletBalance};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeritageWalletMeta {
    #[serde(rename = "wallet_id")]
    pub id: String,
    pub last_sync_ts: u64,
    pub name: String,
    #[serde(default)]
    pub balance: Option<HeritageWalletBalance>,
    #[serde(default)]
    pub block_inclusion_objective: Option<BlockInclusionObjective>,
    #[serde(default)]
    pub fee_rate: Option<FeeRate>,
}
