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
    errors::{Error, ParseBlockInclusionObjectiveError},
    heritage_config::HeritageExplorerTrait,
    subwallet_config::SubwalletId,
    utils::string_to_address,
    HeirConfig, HeritageConfig,
};

/// Balance information for a heritage wallet, split between current and obsolete configurations
///
/// A heritage wallet may contain UTXOs from different heritage configurations over time.
/// This structure tracks balances separately to provide visibility into which funds are
/// tied to the current configuration versus previous configurations.
#[derive(Debug, Clone, Serialize, Deserialize, Default, Eq, PartialEq)]
pub struct HeritageWalletBalance {
    /// Balance tied to the current heritage configuration
    uptodate_balance: Balance,
    /// Balance tied to previous heritage configurations that are no longer current
    obsolete_balance: Balance,
}

impl HeritageWalletBalance {
    /// Creates a new heritage wallet balance
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

/// A transaction recipient with an address and amount
///
/// Represents a destination for bitcoin payments, combining a bitcoin address
/// with the amount to be sent to that address.
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

/// Configuration for how to spend UTXOs in a transaction
///
/// Defines the spending behavior when creating transactions, either by draining
/// all available funds to a single address or distributing to multiple recipients.
#[derive(Debug, Clone)]
pub enum SpendingConfig {
    /// Drain all available funds to a single address
    DrainTo(Address),
    /// Send specific amounts to multiple recipients
    Recipients(Vec<Recipient>),
}
impl SpendingConfig {
    /// Creates a drain-to spending configuration from an address string
    ///
    /// # Errors
    ///
    /// Returns an error if the address string cannot be parsed as a valid bitcoin address
    /// for the configured network.
    ///
    /// # Examples
    ///
    /// ```
    /// # use btc_heritage::utils::bitcoin_network;
    /// # use btc_heritage::bitcoin;
    /// # use bitcoin::Network;
    /// # use btc_heritage::heritage_wallet::SpendingConfig;
    /// // Set the Network to Testnet
    /// bitcoin_network::set(Network::Testnet);
    ///
    /// // This is OK
    /// assert!(SpendingConfig::drain_to_address_str("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").is_ok());
    ///
    /// // This is not
    /// assert!(SpendingConfig::drain_to_address_str("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").is_err());
    /// ```
    pub fn drain_to_address_str(addr: &str) -> crate::errors::Result<SpendingConfig> {
        Ok(SpendingConfig::DrainTo(crate::utils::string_to_address(
            addr,
        )?))
    }
    /// Creates a drain-to spending configuration from an address
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

/// Policy for computing transaction fees
///
/// Defines how transaction fees should be calculated when creating PSBTs.
/// Fees can either be set to an absolute amount or calculated based on
/// transaction size using a fee rate.
#[derive(Debug, Clone)]
pub enum FeePolicy {
    /// Use an exact fee amount regardless of transaction size
    ///
    /// The transaction will have exactly this fee amount, which may result
    /// in a higher or lower fee rate depending on the final transaction size.
    Absolute(Amount),
    /// Calculate fee based on transaction size and the given fee rate
    ///
    /// The final fee amount will be computed as: fee_rate × transaction_weight
    FeeRate(FeeRate),
}

/// UTXO selection strategy for transaction creation
///
/// Controls which UTXOs (Unspent Transaction Outputs) should be considered
/// or excluded when building transactions. This allows fine-grained control
/// over coin selection for privacy, fee optimization, or other purposes.
#[derive(Debug, Clone, Default)]
pub enum UtxoSelection {
    /// Default behavior: include previous UTXOs plus current UTXOs as needed
    ///
    /// Includes all UTXOs from obsolete heritage configurations plus any UTXOs
    /// from the current configuration needed to satisfy the transaction amount.
    /// This maximizes the use of "stale" UTXOs while minimizing current UTXO usage.
    #[default]
    IncludePrevious,
    /// Include specific UTXOs in addition to the default selection
    ///
    /// Behaves like `IncludePrevious` but guarantees the specified UTXOs
    /// are included in the transaction, regardless of whether they're needed
    /// to meet the amount requirement.
    Include(Vec<OutPoint>),
    /// Exclude specific UTXOs from the default selection
    ///
    /// Behaves like `IncludePrevious` but never uses the specified UTXOs,
    /// even if they would otherwise be selected by the algorithm.
    Exclude(HashSet<OutPoint>),
    /// Combination of include and exclude lists
    ///
    /// Like the default behavior, but with explicit inclusion and exclusion
    /// of specific UTXOs. Included UTXOs are guaranteed to be used, excluded
    /// UTXOs are guaranteed not to be used.
    IncludeExclude {
        /// UTXOs that must be included in the transaction
        include: Vec<OutPoint>,
        /// UTXOs that must not be included in the transaction
        exclude: HashSet<OutPoint>,
    },
    /// Use only and all the specified UTXOs, ignore all others
    ///
    /// Only the given UTXOs will be considered for the transaction.
    /// If these UTXOs don't contain sufficient funds, the transaction
    /// creation will fail.
    UseOnly(HashSet<OutPoint>),
}

/// Options for customizing PSBT creation behavior
///
/// Provides fine-grained control over transaction creation including fee policies,
/// UTXO selection strategies, time assumptions, and transaction flags.
#[derive(Debug, Clone, Default)]
pub struct CreatePsbtOptions {
    /// Fee calculation policy for the transaction
    ///
    /// If `None`, uses the wallet's default fee rate based on current
    /// network conditions and block inclusion objective.
    pub fee_policy: Option<FeePolicy>,
    /// Override the current blockchain time for heritage condition evaluation
    ///
    /// Primarily used for testing, this allows simulating future blockchain
    /// states to test time-locked heritage conditions. If `None`, uses the
    /// wallet's last synchronization time as "present".
    pub assume_blocktime: Option<BlockTime>,
    /// UTXO selection strategy for the transaction
    ///
    /// Controls which UTXOs are considered when building the transaction.
    /// Defaults to [`UtxoSelection::IncludePrevious`].
    pub utxo_selection: UtxoSelection,
    /// Disable Replace-By-Fee (RBF) signaling for this transaction
    ///
    /// When `true`, the transaction will not signal RBF support (sequence numbers
    /// will be set to 0xfffffffe). When `false` (default), RBF is enabled.
    ///
    /// **Note**: Since Bitcoin Core v28, full-RBF is the default node configuration,
    /// making this flag largely ineffective for actual replacement prevention.
    pub disable_rbf: bool,
}

/// Block inclusion target for fee estimation
///
/// Represents the number of blocks within which we want a transaction to be included
/// in the blockchain. This value is used when querying fee estimates from Bitcoin Core RPC.
///
/// Lower values (1-6 blocks) result in higher fee rates for faster confirmation.
/// Higher values (100+ blocks) result in lower fee rates but slower confirmation.
///
/// # Valid Range
///
/// Must be between 1 and 1008 blocks (per Bitcoin Core's `estimatesmartfee` RPC).
/// Values outside this range will be clamped to the valid range.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct BlockInclusionObjective(pub(crate) u16);
impl BlockInclusionObjective {
    pub const MIN: Self = BlockInclusionObjective(1);
    pub const MAX: Self = BlockInclusionObjective(1008);
}
impl Default for BlockInclusionObjective {
    /// We arbitrarly choose to make the default value `6 blocks` (1 hour)
    fn default() -> Self {
        Self(6)
    }
}
impl From<u16> for BlockInclusionObjective {
    /// Create a [BlockInclusionObjective] from a [u16]
    /// The result is clamped to ensure its validity
    fn from(value: u16) -> Self {
        Self(value).clamp(Self::MIN, Self::MAX)
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
impl FromStr for BlockInclusionObjective {
    type Err = ParseBlockInclusionObjectiveError;

    /// Creates a [BlockInclusionObjective] from a string representation
    ///
    /// Attempts to parse the string as a [u16] and validates it's within the valid range
    /// (1-1008 blocks).
    ///
    /// # Errors
    ///
    /// Returns [ParseBlockInclusionObjectiveError::InvalidInt] if the string cannot be parsed as a u16.
    /// Returns [ParseBlockInclusionObjectiveError::ValueTooLow] if the value is less than 1.
    /// Returns [ParseBlockInclusionObjectiveError::ValueTooHigh] if the value is greater than 1008.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::str::FromStr;
    /// # use btc_heritage::BlockInclusionObjective;
    ///
    /// let bio = BlockInclusionObjective::from_str("10").unwrap();
    /// assert_eq!(u16::from(bio), 10);
    ///
    /// // Values outside valid range return errors
    /// assert!(BlockInclusionObjective::from_str("2000").is_err());
    /// assert!(BlockInclusionObjective::from_str("0").is_err());
    ///
    /// // Invalid strings return errors
    /// assert!(BlockInclusionObjective::from_str("invalid").is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let u_val = s
            .parse::<u16>()
            .map_err(|_| ParseBlockInclusionObjectiveError::InvalidInt)?;
        if u_val < Self::MIN.0 {
            Err(ParseBlockInclusionObjectiveError::ValueTooLow)
        } else if u_val > Self::MAX.0 {
            Err(ParseBlockInclusionObjectiveError::ValueTooHigh)
        } else {
            Ok(Self(u_val))
        }
    }
}

/// Identifier for a specific subwallet configuration
///
/// Used to reference either the current active configuration or a specific
/// historical configuration by its unique identifier.
#[derive(Debug, Clone, Copy)]
pub enum SubwalletConfigId {
    /// Reference to the current active subwallet configuration
    Current,
    /// Reference to a specific subwallet configuration by ID
    Id(SubwalletId),
}

/// A Bitcoin address that has been validated against the configured network
///
/// This is a wrapper around [`Address<NetworkChecked>`] that automatically validates
/// addresses using the `BITCOIN_NETWORK` environment variable. If the environment
/// variable is not set, it defaults to [`bitcoin::Network::Bitcoin`].
///
/// The address validation ensures that:
/// - The address format is valid for the configured network
/// - Mainnet addresses aren't used on testnet and vice versa
/// - The address can be safely used for transactions
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
            Address::from_script(value, crate::utils::bitcoin_network::get())
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

/// A UTXO (Unspent Transaction Output) in a heritage wallet
///
/// Represents a spendable output that belongs to the heritage wallet, containing
/// all the information needed to spend it including the heritage configuration
/// that controls its spending conditions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
pub struct HeritageUtxo {
    /// [OutPoint] of this UTXO
    pub outpoint: OutPoint,
    /// [Amount] of this UTXO
    #[serde(with = "crate::bitcoin::amount::serde::as_sat")]
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
    /// Estimate the Unix timestamp at which the given [HeirConfig] will be able to spend this [HeritageUtxo].
    ///
    /// This method calculates when an heir can claim inheritance by considering both absolute
    /// and relative time locks defined in the heritage configuration. The calculation takes
    /// into account:
    ///
    /// - **Absolute timelock**: A fixed timestamp when spending becomes possible
    /// - **Relative timelock**: A number of blocks that must pass after UTXO confirmation
    /// - **UTXO confirmation status**: Uses current time for unconfirmed UTXOs
    ///
    /// The heir can spend the UTXO when **both** conditions are met (whichever is later).
    ///
    /// # Parameters
    ///
    /// * `heir_config` - The heir's configuration containing their public key information
    /// * `current_block_time` - Optional current blockchain state for more accurate calculations
    ///
    /// # Returns
    ///
    /// * `Some(timestamp)` - Unix timestamp when the heir can spend this UTXO
    /// * `None` - If the heir is not present in the [HeritageConfig]
    ///
    /// # Note
    ///
    /// The returned timestamp is always an **estimation** for relative timelocks, as it uses
    /// the average Bitcoin block time (approximately 10 minutes) to convert block-based
    /// delays into time-based estimates. Actual spending availability may vary depending
    /// on actual block production times.
    pub fn estimate_heir_spending_timestamp(
        &self,
        heir_config: &HeirConfig,
        current_block_time: Option<BlockTime>,
    ) -> Option<u64> {
        self.heritage_config
            .get_heritage_explorer(heir_config)
            .map(|explo| {
                let spend_conditions = explo.get_spend_conditions();
                let spend_ts = spend_conditions
                    .get_spendable_timestamp()
                    .expect("an Heir always have a timelock in v1");
                let relative_block_lock = spend_conditions
                    .get_relative_block_lock()
                    .expect("an Heir always have a relative_block_lock in v1");

                let (reference_timestamp, missing_blocks) =
                    match (self.confirmation_time.clone(), current_block_time) {
                        // Both UTXO confirmation and current blockchain state are known
                        // Calculate exactly how many blocks are remaining
                        (
                            Some(BlockTime {
                                height: confirmation_height,
                                timestamp: confirmation_timestamp,
                            }),
                            Some(BlockTime {
                                height: current_height,
                                ..
                            }),
                        ) => {
                            // Calculate blocks still needed after current height for relative lock to expire
                            let missing_blocks = (confirmation_height + relative_block_lock as u32)
                                .checked_sub(current_height)
                                .unwrap_or(0);
                            (confirmation_timestamp, missing_blocks)
                        }
                        // UTXO is confirmed but current blockchain state is unknown
                        // Use confirmation time as reference point and do as if it was the present
                        (
                            Some(BlockTime {
                                timestamp: confirmation_timestamp,
                                ..
                            }),
                            None,
                        ) => {
                            // Use full relative lock period from confirmation time
                            (confirmation_timestamp, relative_block_lock as u32)
                        }
                        // UTXO is unconfirmed (with or without current blockchain state)
                        // Use current time as optimistic baseline
                        (None, Some(_)) | (None, None) => {
                            // Assume immediate confirmation and apply full relative lock period
                            (crate::utils::timestamp_now(), relative_block_lock as u32)
                        }
                    };

                // Estimate timestamp when relative timelock expires using average block time
                let relative_lock_ts_estimate = reference_timestamp
                    + crate::utils::AVERAGE_BLOCK_TIME_SEC as u64 * missing_blocks as u64;

                // Return the later of absolute timelock or estimated relative timelock
                spend_ts.max(relative_lock_ts_estimate)
            })
    }
}

/// Summary totals for transaction inputs or outputs
///
/// Aggregates the count and total amount for either inputs or outputs
/// in a transaction summary.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionSummaryIOTotals {
    /// Number of inputs or outputs
    pub count: usize,
    /// Total amount of all inputs or outputs
    #[serde(with = "crate::bitcoin::amount::serde::as_sat")]
    pub amount: Amount,
}
impl TransactionSummaryIOTotals {
    /// Increments the count and adds the given amount to the total
    ///
    /// This method is used to track both the number of inputs/outputs
    /// and their cumulative amount in a transaction summary.
    ///
    /// # Examples
    ///
    /// ```
    /// # use btc_heritage::heritage_wallet::TransactionSummaryIOTotals;
    /// use btc_heritage::bitcoin::Amount;
    /// let mut totals = TransactionSummaryIOTotals::default();
    /// totals.count_io_amount(Amount::from_sat(1000));
    /// assert_eq!(totals.count, 1);
    /// assert_eq!(totals.amount, Amount::from_sat(1000));
    /// ```
    pub fn count_io_amount(&mut self, amount: Amount) {
        self.count += 1;
        self.amount += amount;
    }
}

/// Information about a transaction input or output owned by the wallet
///
/// Represents either an input being spent from or an output being received by
/// addresses controlled by the heritage wallet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionSummaryOwnedIO {
    /// The outpoint being spent (for inputs) or created (for outputs)
    pub outpoint: OutPoint,
    /// The address associated with this input/output
    pub address: CheckedAddress,
    /// The amount being spent or received
    #[serde(with = "crate::bitcoin::amount::serde::as_sat")]
    pub amount: Amount,
}

/// Summary of a wallet transaction with owned inputs and outputs
///
/// Provides a comprehensive view of a transaction that involves the heritage wallet,
/// including which inputs and outputs belong to the wallet, fee information,
/// and confirmation status. This is used for transaction history and analysis.
///
/// The summary distinguishes between:
/// - **Owned** inputs/outputs: Those belonging to addresses controlled by this wallet
/// - **Total** inputs/outputs: All inputs/outputs in the transaction, including external ones
///
/// This distinction allows users to understand both their involvement in the transaction
/// and the transaction's overall structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TransactionSummary {
    /// Unique transaction identifier
    pub txid: Txid,
    /// Confirmation status and timestamp
    ///
    /// Contains block height and Unix timestamp if confirmed, `None` if still pending.
    #[serde(default, flatten, skip_serializing_if = "Option::is_none")]
    pub confirmation_time: Option<BlockTime>,
    /// Inputs owned by this wallet that were spent in this transaction
    ///
    /// Each entry represents a UTXO that was previously controlled by this wallet
    /// and was consumed as an input in this transaction.
    pub owned_inputs: Vec<TransactionSummaryOwnedIO>,
    /// Total count and amount of all inputs in this transaction
    ///
    /// Includes both owned and external inputs for complete transaction context.
    pub inputs_totals: TransactionSummaryIOTotals,
    /// Outputs owned by this wallet that were created by this transaction
    ///
    /// Each entry represents a new UTXO that this wallet can spend, created
    /// as an output of this transaction.
    pub owned_outputs: Vec<TransactionSummaryOwnedIO>,
    /// Total count and amount of all outputs in this transaction
    ///
    /// Includes both owned and external outputs for complete transaction context.
    pub outputs_totals: TransactionSummaryIOTotals,
    /// Transaction fee paid in satoshis
    ///
    /// Calculated as the difference between total input and output amounts.
    #[serde(with = "crate::bitcoin::amount::serde::as_sat")]
    pub fee: Amount,
    /// Fee rate in satoshis per thousand weight units (sat/kWU)
    ///
    /// Calculated as: fee / (transaction_weight / 1000)
    pub fee_rate: FeeRate,
    /// Transaction dependencies within the same block
    ///
    /// Contains TXIDs of transactions in the same block that this transaction
    /// depends on, used for proper transaction ordering in the wallet history (CLI/GUI/).
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

/// A bitcoin address with its HD wallet derivation information
///
/// Combines a bitcoin address with the key fingerprint and derivation path
/// that was used to generate it, enabling wallet backup and recovery.
/// The address is automatically validated against the configured network.
///
/// # Format
///
/// When serialized to string, uses the format: `[<fingerprint>/<derivation_path>]<address>`
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(into = "String", try_from = "String")]
pub struct WalletAddress {
    pub(crate) origin: (Fingerprint, DerivationPath),
    pub(crate) address: Address<NetworkChecked>,
}
impl WalletAddress {
    /// Returns the key origin information (fingerprint and derivation path)
    ///
    /// The fingerprint identifies the master key, and the derivation path
    /// shows how to derive the specific key used for this address.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use btc_heritage::heritage_wallet::WalletAddress;
    /// # let wallet_address: WalletAddress = todo!();
    /// let (fingerprint, derivation_path) = wallet_address.origin();
    /// println!("Key fingerprint: {}", fingerprint);
    /// println!("Derivation path: {}", derivation_path);
    /// ```
    pub fn origin(&self) -> &(Fingerprint, DerivationPath) {
        &self.origin
    }

    /// Returns the bitcoin address
    pub fn address(&self) -> &Address<NetworkChecked> {
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

        let derivation_path = DerivationPath::from_str(&format!("m/{derivation_path_str}"))
            .map_err(|e| {
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
        let origin_str = self.origin.1.to_string();
        // origin_str = m/[...]
        // origin_str[2..] strips the m/
        write!(
            f,
            "[{}/{}]{}",
            self.origin.0,
            &origin_str[2..],
            self.address
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::{
        get_default_test_subwallet_config_expected_address,
        get_default_test_subwallet_config_expected_address_without_origin, TestHeritageConfig,
    };

    use super::*;
    use serde_json;

    #[test]
    fn test_checked_address_serialization() {
        let addr_str = get_default_test_subwallet_config_expected_address_without_origin(
            TestHeritageConfig::BackupWifeY2,
            0,
        );
        let checked_addr = CheckedAddress::try_from(addr_str).unwrap();

        // Test serialization to JSON
        let serialized = serde_json::to_string(&checked_addr).unwrap();
        assert_eq!(serialized, format!("\"{}\"", addr_str));

        // Test deserialization from JSON
        let deserialized: CheckedAddress = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, checked_addr);
    }

    #[test]
    fn test_wallet_address_serialization() {
        let addr_str =
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 0);
        let wallet_addr = WalletAddress::try_from(addr_str).unwrap();

        // Test serialization to JSON
        let serialized = serde_json::to_string(&wallet_addr).unwrap();
        assert_eq!(serialized, format!("\"{}\"", addr_str));

        // Test deserialization from JSON
        let deserialized: WalletAddress = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, wallet_addr);
    }
}
