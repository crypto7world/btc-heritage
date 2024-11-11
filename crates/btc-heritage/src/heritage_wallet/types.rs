use core::{fmt::Display, ops::Deref, str::FromStr};
use std::collections::HashSet;

use bdk::{
    bitcoin::{FeeRate, Script, ScriptBuf},
    Balance, BlockTime,
};
use serde::{Deserialize, Serialize};

use crate::{
    bitcoin::{
        address::NetworkChecked,
        bip32::{DerivationPath, Fingerprint},
        Address, Amount, OutPoint, Txid,
    },
    errors::Error,
    heritage_config::HeritageExplorerTrait,
    subwallet_config::SubwalletId,
    utils::string_to_address,
    HeirConfig, HeritageConfig,
};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
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

/// The policy to compute the fee of a new transaction
#[derive(Debug, Clone)]
pub enum FeePolicy {
    /// The new transaction will have the exact fee amount
    Absolute(Amount),
    /// The new transaction will use the given fee rate to compute the fee
    FeeRate(FeeRate),
}

/// The UTXO selection mode
#[derive(Debug, Clone, Default)]
pub enum UtxoSelection {
    /// Default behavior,
    /// includes all the 'previous' UTXOs (bound to non-current Heritage Configs),
    /// plus the 'current' UTXOs needed to match the requested amount, if any are necessary
    #[default]
    IncludePrevious,
    /// Like the default behavior, plus always include the given UTXOs
    Include(Vec<OutPoint>),
    /// Like the default behavior, but always exclude the given UTXOs
    Exclude(HashSet<OutPoint>),
    /// Combinaison of Include and Exclude: like the default behavior, but include and exclude the given UTXOs
    IncludeExclude {
        include: Vec<OutPoint>,
        exclude: HashSet<OutPoint>,
    },
    /// Use all the given UTXOs, and only the given UTXOs
    UseOnly(HashSet<OutPoint>),
}

/// Options used to customize the behavior of [super::HeritageWallet::create_psbt]
#[derive(Debug, Clone, Default)]
pub struct CreatePsbtOptions {
    pub fee_policy: Option<FeePolicy>,
    pub assume_blocktime: Option<BlockTime>,
    pub utxo_selection: UtxoSelection,
}

/// An [HeritageWallet] configuration used to query the appropriate [crate::bitcoin::FeeRate]
/// from BitcoinCore RPC. It represents the number of blocks we are willing to wait before a
/// transaction is included in the blockchain. Per https://developer.bitcoin.org/reference/rpc/estimatesmartfee.html
/// it must be between 1 and 1008.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
#[serde(transparent)]
pub struct BlockInclusionObjective(pub(crate) u16);
impl Default for BlockInclusionObjective {
    /// We arbitrarly choose to make the default value `6 blocks` (1 hour)
    fn default() -> Self {
        Self(6)
    }
}
impl From<u16> for BlockInclusionObjective {
    /// Create a [BlockInclusionObjective] from a value that can be converted into a [u16]
    ///
    /// # Panics
    /// Panics if the resulting internal [u16] is less than 1 or more than 1008
    fn from(value: u16) -> Self {
        let bio: u16 = value.into();
        assert!(1 <= bio && bio <= 1008);
        Self(bio)
    }
}
impl From<BlockInclusionObjective> for u16 {
    fn from(value: BlockInclusionObjective) -> Self {
        value.0
    }
}
impl Display for BlockInclusionObjective {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum SubwalletConfigId {
    Current,
    Id(SubwalletId),
}

/// Wrapper around an [Address<NetworkChecked>] that automatically check the address
/// using the `BITCOIN_NETWORK` environment variable.
/// If the environment variable is absent, assume [crate::bitcoin::Network::Bitcoin]
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(into = "String", try_from = "String")]
pub struct CheckedAddress(Address<NetworkChecked>);
impl Deref for CheckedAddress {
    type Target = Address<NetworkChecked>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<Address<NetworkChecked>> for CheckedAddress {
    fn from(value: Address<NetworkChecked>) -> Self {
        Self(value)
    }
}
impl TryFrom<String> for CheckedAddress {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        Self::try_from(value.as_str())
    }
}
impl TryFrom<&Script> for CheckedAddress {
    type Error = Error;
    fn try_from(value: &Script) -> Result<Self, Error> {
        Ok(Self::from(
            Address::from_script(value, *crate::utils::bitcoin_network_from_env())
                .map_err(|e| Error::Unknown(format!("Invalid script: {e}")))?,
        ))
    }
}
impl TryFrom<&ScriptBuf> for CheckedAddress {
    type Error = Error;
    fn try_from(value: &ScriptBuf) -> Result<Self, Error> {
        Self::try_from(value.as_script())
    }
}
impl TryFrom<&str> for CheckedAddress {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Error> {
        Ok(Self(string_to_address(value)?))
    }
}

impl From<CheckedAddress> for String {
    fn from(value: CheckedAddress) -> String {
        value.to_string()
    }
}
impl Display for CheckedAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
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
    /// The Bitcoin [CheckedAddress] of this UTXO
    pub address: CheckedAddress,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionSummaryOwnedIO {
    pub outpoint: OutPoint,
    pub address: CheckedAddress,
    #[serde(with = "crate::utils::amount_serde")]
    pub amount: Amount,
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
    /// The owned inputs addresses and amounts of this transaction
    pub owned_inputs: Vec<TransactionSummaryOwnedIO>,
    /// The owned outputs addresses and amounts of this transaction
    pub owned_outputs: Vec<TransactionSummaryOwnedIO>,
    /// Fee value (sats)
    #[serde(with = "crate::utils::amount_serde")]
    pub fee: Amount,
    /// Fee rate (sat/kWU)
    pub fee_rate: FeeRate,
    /// The previous [Txid] of the same block on which this transaction depends. For ordering purposes
    pub parent_txids: HashSet<Txid>,
}

// /// A descriptors backup to export an HeritageWallet configuration
// #[derive(Debug, Clone, Serialize, Deserialize)]
// #[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
// pub struct DescriptorsBackup {
//     pub external_descriptor: String,
//     pub change_descriptor: String,
//     pub first_use_ts: Option<u64>,
//     pub last_external_index: Option<u32>,
//     pub last_change_index: Option<u32>,
// }

/// A [Address<NetworkChecked>] with [(Fingerprint, DerivationPath)] informations
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
#[serde(into = "String", try_from = "String")]
pub struct WalletAddress {
    pub(crate) origin: (Fingerprint, DerivationPath),
    pub(crate) address: Address<NetworkChecked>,
}
impl WalletAddress {
    pub fn origin(&self) -> &(Fingerprint, DerivationPath) {
        &self.origin
    }
    pub fn address(&self) -> &Address {
        &self.address
    }
}
impl Deref for WalletAddress {
    type Target = Address<NetworkChecked>;

    fn deref(&self) -> &Self::Target {
        &self.address
    }
}
impl From<((Fingerprint, DerivationPath), Address<NetworkChecked>)> for WalletAddress {
    fn from(value: ((Fingerprint, DerivationPath), Address<NetworkChecked>)) -> Self {
        Self {
            origin: value.0,
            address: value.1,
        }
    }
}
impl TryFrom<String> for WalletAddress {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        Self::try_from(value.as_str())
    }
}
impl TryFrom<&str> for WalletAddress {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Error> {
        // Expected format: [<fingerprint>/<derivation_path>]<address>
        let error_c = || Error::InvalidWalletAddressString(value.to_owned());

        // Strip off the opening bracket and split at the closing one
        let mut parts = value.strip_prefix('[').ok_or_else(error_c)?.split(']');

        // Extract the origin, the first part
        let origin_str = parts.next().ok_or_else(error_c)?;
        // Extract the address, the second part
        let address_str = parts.next().ok_or_else(error_c)?;
        // There is no other part
        let None = parts.next() else {
            return Err(error_c());
        };
        // Split the origin into <fingerprint>/<derivation_path>
        let Some((fingerprint_str, derivation_path_str)) = origin_str.split_once('/') else {
            return Err(error_c());
        };
        let fingerprint = Fingerprint::from_str(fingerprint_str).map_err(|e| {
            log::error!("Could not parse fingerprint: {fingerprint_str} ({e})");
            error_c()
        })?;
        let derivation_path = DerivationPath::from_str(derivation_path_str).map_err(|e| {
            log::error!("Could not parse derivation_path: {derivation_path_str} ({e})");
            error_c()
        })?;
        let address = string_to_address(address_str).map_err(|e| {
            log::error!("Could not parse address: {address_str} ({e})");
            error_c()
        })?;

        Ok(Self {
            origin: (fingerprint, derivation_path),
            address,
        })
    }
}

impl From<WalletAddress> for String {
    fn from(value: WalletAddress) -> String {
        value.to_string()
    }
}
impl Display for WalletAddress {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "[{}/{}]{}", self.origin.0, self.origin.1, self.address)
    }
}
