use core::{ops::Deref, str::FromStr};
use std::collections::BTreeSet;

use btc_heritage::{
    bitcoin::OutPoint,
    heritage_wallet::{FeePolicy, UtxoSelection},
    subwallet_config::SubwalletConfig,
    Amount, HeirConfig,
};
use serde::{Deserialize, Serialize};

// Expose API types
pub use btc_heritage::{
    bitcoin::{bip32::Fingerprint, FeeRate, Txid},
    heritage_wallet::{HeritageUtxo, TransactionSummary, TransactionSummaryOwnedIO},
    AccountXPub, AccountXPubId, BlockInclusionObjective, HeritageConfig, HeritageWalletBackup,
    HeritageWalletBalance, PartiallySignedTransaction,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeritageWalletMeta {
    #[serde(rename = "wallet_id")]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<Fingerprint>,
    pub last_sync_ts: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<HeritageWalletBalance>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_inclusion_objective: Option<BlockInclusionObjective>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_rate: Option<FeeRate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeritageWalletMetaCreate {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<HeritageWalletBackup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_inclusion_objective: Option<BlockInclusionObjective>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HeritageWalletMetaUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub block_inclusion_objective: Option<BlockInclusionObjective>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "accountxpub",
    rename_all = "SCREAMING_SNAKE_CASE"
)]
pub enum AccountXPubWithStatus {
    Used(AccountXPub),
    Unused(AccountXPub),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubwalletConfigMeta {
    pub account_xpub: AccountXPub,
    pub heritage_config: HeritageConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub firstuse_ts: Option<u64>,
}
impl From<SubwalletConfig> for SubwalletConfigMeta {
    fn from(value: SubwalletConfig) -> Self {
        let firstuse_ts = value.subwallet_firstuse_time();
        let (account_xpub, heritage_config) = value.into_parts();
        Self {
            account_xpub,
            heritage_config,
            firstuse_ts,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewTxRecipient {
    pub address: String,
    pub amount: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NewTxSpendingConfig {
    Recipients(Vec<NewTxRecipient>),
    DrainTo(NewTxDrainTo),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NewTxFeePolicy {
    /// Amount in sat
    Absolute { amount: u64 },
    /// Rate in sat/vB
    Rate { rate: f32 },
}
impl From<NewTxFeePolicy> for FeePolicy {
    fn from(value: NewTxFeePolicy) -> Self {
        match value {
            NewTxFeePolicy::Absolute { amount } => FeePolicy::Absolute(Amount::from_sat(amount)),
            NewTxFeePolicy::Rate { rate } => {
                // rate is in sat/vB and we have to convert it to sat/kWU
                // 1 vB = 4 WU
                // 1 vB = 0.004 kWU
                // 1 sat/vB = 1/0.004 sat/kWU
                // 1 sat/vB = 250 sat/kWU
                FeePolicy::FeeRate(FeeRate::from_sat_per_kwu(
                    (rate * 250.0).min(u64::MAX as f32) as u64,
                ))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum NewTxUtxoSelection {
    IncludeExclude {
        include: Vec<OutPoint>,
        exclude: Vec<OutPoint>,
    },
    Include {
        include: Vec<OutPoint>,
    },
    Exclude {
        exclude: Vec<OutPoint>,
    },
    UseOnly {
        use_only: Vec<OutPoint>,
    },
}

impl From<NewTxUtxoSelection> for UtxoSelection {
    fn from(value: NewTxUtxoSelection) -> Self {
        match value {
            NewTxUtxoSelection::Include { include } => UtxoSelection::Include(include),
            NewTxUtxoSelection::Exclude { exclude } => {
                UtxoSelection::Exclude(exclude.into_iter().collect())
            }
            NewTxUtxoSelection::IncludeExclude { include, exclude } => {
                UtxoSelection::IncludeExclude {
                    include,
                    exclude: exclude.into_iter().collect(),
                }
            }
            NewTxUtxoSelection::UseOnly { use_only } => {
                UtxoSelection::UseOnly(use_only.into_iter().collect())
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewTx {
    pub spending_config: NewTxSpendingConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_policy: Option<NewTxFeePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub utxo_selection: Option<NewTxUtxoSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disable_rbf: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NewTxDrainTo {
    pub drain_to: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SynchronizationStatus {
    #[default]
    Never,
    Queued,
    InProgress,
    Ok,
    Failed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct Synchronization {
    #[serde(default)]
    pub status: SynchronizationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued_ts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_ts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ts: Option<u64>,
}

/// Created from an HeritageUtxo for each heir_config in the HeritageConfig
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heritage {
    /// The identifier of this concrete heritage, a hash of user_id || wallet_id || heir_id.
    /// It is not unique and will be used as a DynamoDB index marker for iHeritages
    /// Serve two purposes:
    /// - a user can request a PSBT using this ID and this object will allow to easily find the HeritageWallet
    /// - the service can scan the iHeritages index to find every ConcretHeritage, thus easily verifying
    /// expirations and notifying users
    pub heritage_id: String,
    /// The heir_config for which the following info are generated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heir_config: Option<HeirConfig>,
    /// The email of the owner of the HeritageWallet from which the ConcreteHeritage comes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_email: Option<String>,
    /// The value of this ConcretHeritage (correspond to the underlying HeritageUTXO value)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<u64>,
    /// The timestamp after which the Heir is able to spend
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maturity: Option<u64>,
    /// The maturity of the next heir, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_heir_maturity: Option<Option<u64>>,
    /// The position of the heir in the HeritageConfig
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heir_position: Option<u8>,
    /// The number of heirs in the HeritageConfig
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heirs_count: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct EmailAddress(String);
impl std::fmt::Display for EmailAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl TryFrom<&str> for EmailAddress {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        EmailAddress::try_from(value.to_owned())
    }
}

fn re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^[a-zA-Z0-9]([a-zA-Z0-9._-]*[a-zA-Z0-9])*@([a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])*\.)+[a-zA-Z]{2,}$").unwrap()
    })
}
impl TryFrom<String> for EmailAddress {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if re().is_match(&value) {
            Ok(Self(value))
        } else {
            Err(format!("{value} is not a valid Email address"))
        }
    }
}
impl Deref for EmailAddress {
    type Target = String;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HeirContact {
    Email { email: EmailAddress },
    // TODO Phone(String),
}

/// An enum telling what the Heir can know about its inheritence before they
/// reach the point in time when they can actually spend the funds.
///
/// Note that in case an heritage goes to maturity for some reason, the heir will be
/// notified using its contact informations and they will also have all view permissions
/// on the specific HeritageConfig expect the ability to see who are the other heirs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HeirPermission {
    /// The Heir can see the inheritance
    IsHeir,
    /// The Heir can see the email of the owner
    OwnerEmail,
    /// The Heir can see the amount they may inherit
    Amount,
    /// The Heir can see the maturity dates of their inheritance
    Maturity,
    /// The heir can see their position in the Heritage Configurations and the total number of heirs
    Position,
    /// [Only valid AFTER maturity] The heir can see the full Bitcoin Descriptors of the Heritage Configurations.
    /// Mandatory if the Heir's wallet is a Ledger, as it will request the full descriptor before spending.
    /// Note that it means the heir will be able to infer the public keys of other heirs and their own position.
    FullDescriptor,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HeirPermissions(BTreeSet<HeirPermission>);
impl Deref for HeirPermissions {
    type Target = BTreeSet<HeirPermission>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: IntoIterator<Item = HeirPermission>> From<T> for HeirPermissions {
    fn from(value: T) -> Self {
        HeirPermissions(BTreeSet::from_iter(value))
    }
}
impl HeirPermissions {
    pub fn normalize(&mut self) {
        // // IsHeir cannot be the only permission
        // if self.0.len() == 1 && self.0.contains(&HeirPermission::IsHeir) {
        //     self.0.remove(&HeirPermission::IsHeir);
        // }

        // Do nothing
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MainContact {
    pub email: EmailAddress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Heir {
    #[serde(rename = "heir_id")]
    pub id: String,
    pub display_name: String,
    pub heir_config: HeirConfig,
    #[serde(flatten)]
    pub main_contact: MainContact,
    pub permissions: HeirPermissions,
    pub additional_contacts: BTreeSet<HeirContact>,
    pub owner_email: EmailAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeirCreate {
    pub display_name: String,
    pub heir_config: HeirConfig,
    #[serde(flatten)]
    pub main_contact: MainContact,
    #[serde(default)]
    pub permissions: HeirPermissions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeirUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", flatten)]
    pub main_contact: Option<MainContact>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<HeirPermissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
struct StringPsbt(PartiallySignedTransaction);
impl TryFrom<String> for StringPsbt {
    type Error = <PartiallySignedTransaction as FromStr>::Err;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(StringPsbt(PartiallySignedTransaction::from_str(&value)?))
    }
}
impl From<StringPsbt> for String {
    fn from(value: StringPsbt) -> Self {
        value.0.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct UnsignedPsbt {
    psbt: StringPsbt,
    tx_summary: TransactionSummary,
}

impl From<(PartiallySignedTransaction, TransactionSummary)> for UnsignedPsbt {
    fn from(value: (PartiallySignedTransaction, TransactionSummary)) -> Self {
        let (psbt, tx_summary) = value;
        Self {
            psbt: StringPsbt(psbt),
            tx_summary,
        }
    }
}

impl From<UnsignedPsbt> for (PartiallySignedTransaction, TransactionSummary) {
    fn from(value: UnsignedPsbt) -> Self {
        let UnsignedPsbt {
            psbt: StringPsbt(psbt),
            tx_summary,
        } = value;
        (psbt, tx_summary)
    }
}
