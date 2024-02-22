use std::fmt::Display;

use bdk::{
    bitcoin::{Address, Amount, OutPoint, Txid},
    Balance, BlockTime,
};
use serde::{Deserialize, Serialize};

use crate::{
    errors::Error, heritageconfig::HeritageExplorerTrait, subwalletconfig::SubwalletId, HeirConfig,
    HeritageConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct HeritageWalletBalance {
    uptodate_balance: Balance,
    obsolete_balance: Balance,
}

impl HeritageWalletBalance {
    pub fn new(uptodate_balance: Balance, obsolete_balance: Balance) -> Self {
        Self {
            uptodate_balance,
            obsolete_balance,
        }
    }
    /// The balance of the [HeritageWallet], regardless of it being tied to up-to-date or obsolete [HeritageConfig]
    pub fn total_balance(&self) -> Balance {
        Balance {
            immature: self.uptodate_balance.immature + self.obsolete_balance.immature,
            trusted_pending: self.uptodate_balance.trusted_pending
                + self.obsolete_balance.trusted_pending,
            untrusted_pending: self.uptodate_balance.untrusted_pending
                + self.obsolete_balance.untrusted_pending,
            confirmed: self.uptodate_balance.confirmed + self.obsolete_balance.confirmed,
        }
    }

    /// The balance tied to the wallet using the current `HeritageConfig`
    pub fn uptodate_balance(&self) -> &Balance {
        &self.uptodate_balance
    }

    /// The balance tied to the wallet(s) using previous `HeritageConfig`
    pub fn obsolete_balance(&self) -> &Balance {
        &self.obsolete_balance
    }
}

#[derive(Debug, Clone)]
pub struct Recipient(pub(crate) Address, pub(crate) Amount);
impl From<(Address, Amount)> for Recipient {
    fn from(value: (Address, Amount)) -> Self {
        Self(value.0, value.1)
    }
}
impl TryFrom<(&str, Amount)> for Recipient {
    type Error = Error;

    fn try_from(value: (&str, Amount)) -> Result<Self, Self::Error> {
        let (addr_str, amount) = value;
        let addr = crate::utils::string_to_address(addr_str)?;
        Ok(Self(addr, amount))
    }
}
impl TryFrom<(String, Amount)> for Recipient {
    type Error = Error;

    fn try_from(value: (String, Amount)) -> Result<Self, Self::Error> {
        let (addr_str, amount) = value;
        Self::try_from((addr_str.as_str(), amount))
    }
}

#[derive(Debug, Clone)]
pub enum SpendingConfig {
    DrainTo(Address),
    Recipients(Vec<Recipient>),
}
impl SpendingConfig {
    pub fn drain_to_address_str(addr: &str) -> crate::errors::Result<SpendingConfig> {
        Ok(SpendingConfig::DrainTo(crate::utils::string_to_address(
            addr,
        )?))
    }
    pub fn drain_to_address(addr: Address) -> SpendingConfig {
        SpendingConfig::DrainTo(addr)
    }
}
impl From<Vec<(Address, Amount)>> for SpendingConfig {
    fn from(value: Vec<(Address, Amount)>) -> Self {
        SpendingConfig::Recipients(value.into_iter().map(|e| Recipient::from(e)).collect())
    }
}
impl TryFrom<Vec<(String, Amount)>> for SpendingConfig {
    type Error = Error;

    fn try_from(value: Vec<(String, Amount)>) -> Result<Self, Self::Error> {
        Ok(SpendingConfig::Recipients(
            value
                .into_iter()
                .map(|e| Recipient::try_from(e))
                .collect::<Result<_, _>>()?,
        ))
    }
}

/// An [HeritageWallet] configuration used to query the appropriate [bdk::types::FeeRate]
/// from BitcoinCore RPC. It represents the number of blocks we are willing to wait before a
/// transaction is included in the blockchain. Per https://developer.bitcoin.org/reference/rpc/estimatesmartfee.html
/// it must be between 1 and 1008.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct BlockInclusionObjective(pub(crate) u16);
impl Default for BlockInclusionObjective {
    /// We arbitrarly choose to make the default value `6 blocks` (1 hour)
    fn default() -> Self {
        Self(6)
    }
}
impl<T: Into<u16>> From<T> for BlockInclusionObjective {
    /// Create a [BlockInclusionObjective] from a value that can be converted into a [u16]
    ///
    /// # Panics
    /// Panics if the resulting internal [u16] is less than 1 or more than 1008
    fn from(value: T) -> Self {
        let bio: u16 = value.into();
        assert!(1 <= bio && bio <= 1008);
        Self(bio)
    }
}
impl Display for BlockInclusionObjective {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SubwalletConfigId {
    Current,
    Id(SubwalletId),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeritageUtxo {
    /// [OutPoint] of this UTXO
    pub outpoint: OutPoint,
    /// [Amount] of this UTXO
    #[serde(with = "crate::utils::amount_serde")]
    pub amount: Amount,
    /// The [BlockTime] of the block that contains the Tx referenced by the [OutPoint]
    /// Can be None if the UTXO is for a unconfirmed TX
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub confirmation_time: Option<BlockTime>,
    /// The [HeritageConfig] of the subwallet that owns this UTXO
    pub heritage_config: HeritageConfig,
}
impl HeritageUtxo {
    /// Returns the timestamp at which the given [HeirConfig] will be able to spend this [HeritageUtxo].
    /// If the heir is not present the the [HeritageConfig], the function returns [None].
    ///
    /// Beware that this MAY be an estimation based on the average Bitcoin network blocktime.
    pub fn estimate_heir_spending_timestamp(&self, heir_config: &HeirConfig) -> Option<u64> {
        self.heritage_config
            .get_heritage_explorer(heir_config)
            .map(|explo| {
                let spend_conditions = explo.get_spend_conditions();
                let spend_ts = spend_conditions
                    .get_spendable_timestamp()
                    .expect("an Heir always have a timelock");
                let confirmation_timestamp =
                    if let Some(confirmation_time) = &self.confirmation_time {
                        confirmation_time.timestamp
                    } else {
                        // If the UTXO is not yet confirmed, we do as-if it was confirmed now.
                        crate::utils::timestamp_now()
                    };
                if let Some(relative_block_lock) = spend_conditions.get_relative_block_lock() {
                    let relative_lock_ts_estimate = confirmation_timestamp
                        + crate::utils::AVERAGE_BLOCK_TIME_SEC as u64 * relative_block_lock as u64;
                    spend_ts.max(relative_lock_ts_estimate)
                } else {
                    spend_ts
                }
            })
    }
}

/// A wallet transaction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionSummary {
    /// Transaction id
    pub txid: Txid,
    /// If the transaction is confirmed, contains height and Unix timestamp of the block containing the
    /// transaction, unconfirmed transaction contains `None`.
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub confirmation_time: Option<BlockTime>,
    /// Received value (sats)
    /// Sum of owned outputs of this transaction.
    #[serde(with = "crate::utils::amount_serde")]
    pub received: Amount,
    /// Sent value (sats)
    /// Sum of owned inputs of this transaction.
    #[serde(with = "crate::utils::amount_serde")]
    pub sent: Amount,
    /// Fee value (sats)
    #[serde(with = "crate::utils::amount_serde")]
    pub fee: Amount,
}
impl PartialOrd for TransactionSummary {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for TransactionSummary {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.confirmation_time
            .cmp(&other.confirmation_time)
            .then_with(|| self.txid.cmp(&other.txid))
    }
}

/// A descriptors backup to export an HeritageWallet configuration
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DescriptorsBackup {
    pub external_descriptor: String,
    pub change_descriptor: String,
    pub first_use_ts: Option<u64>,
    pub last_external_index: Option<u32>,
    pub last_change_index: Option<u32>,
}
