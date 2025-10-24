use core::{fmt::Debug, str::FromStr};

use serde::{Deserialize, Serialize};

use self::heirtypes::HeirConfig;
use crate::{
    bitcoin::{
        bip32::{DerivationPath, Fingerprint},
        ScriptBuf,
    },
    errors::{Error, Result},
    miniscript::{DefiniteDescriptorKey, Miniscript, Tap},
};

pub mod heirtypes;
pub mod v1;

/// Conditions that must be met before an heir can spend heritage funds
///
/// This struct encapsulates the temporal constraints on when heritage funds
/// become spendable by an heir, including absolute time locks and relative
/// block height locks.
#[derive(Debug, Clone)]
pub struct SpendConditions {
    spendable_timestamp: Option<u64>,
    relative_block_lock: Option<u16>,
}
impl SpendConditions {
    /// Checks if the spending conditions are satisfied at the current time
    pub fn can_spend_now(&self) -> bool {
        let now = crate::utils::timestamp_now();
        self.can_spend_at(now)
    }

    /// Checks if the spending conditions are satisfied at the given timestamp
    ///
    /// # Arguments
    ///
    /// * `ts` - Unix timestamp in seconds to check against
    pub fn can_spend_at(&self, ts: u64) -> bool {
        ts >= self.spendable_timestamp.unwrap_or(0)
    }
    /// Returns the relative block lock if one is set
    ///
    /// The relative block lock specifies how many blocks must pass after inclusion
    /// in the blockchain before the given Heritage UTXO can be spent.
    ///
    /// If `None`, there is no relative block lock restriction.
    pub fn get_relative_block_lock(&self) -> Option<u16> {
        self.relative_block_lock
    }
    /// Returns the absolute timestamp when spending becomes allowed
    ///
    /// If `None`, there is no absolute time restriction.
    pub fn get_spendable_timestamp(&self) -> Option<u64> {
        self.spendable_timestamp
    }
    /// Creates spending conditions for the wallet owner
    ///
    /// Owners have no spending restrictions and can spend immediately.
    pub fn for_owner() -> Self {
        SpendConditions {
            spendable_timestamp: None,
            relative_block_lock: None,
        }
    }
}

/// Configuration defining inheritance rules and heir spending conditions
///
/// A HeritageConfig specifies how and when heirs can inherit funds from a Bitcoin wallet.
/// It contains multiple heritage entries, each with different maturity times and spending
/// conditions. The config is versioned to allow for future upgrades while maintaining
/// backward compatibility.
#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HeritageConfig(InnerHeritageConfig);

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "version", rename_all = "lowercase")]
enum InnerHeritageConfig {
    V1(v1::HeritageConfig),
}

/// Version identifier for HeritageConfig formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeritageConfigVersion {
    V1 = 1,
}
impl FromStr for HeritageConfigVersion {
    type Err = Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "1" | "v1" => Ok(Self::V1),
            _ => Err(Error::InvalidHeritageConfigString(s.to_owned())),
        }
    }
}

impl HeritageConfig {
    /// Returns a builder for the default HeritageConfig version
    ///
    /// Currently defaults to V1. This is the recommended way to create
    /// new HeritageConfig instances.
    pub fn builder() -> v1::HeritageConfigBuilder {
        HeritageConfig::builder_v1()
    }

    /// Returns a builder specifically for HeritageConfig V1
    pub fn builder_v1() -> v1::HeritageConfigBuilder {
        v1::HeritageConfig::builder()
    }

    /// Returns the version of this HeritageConfig
    pub fn version(&self) -> HeritageConfigVersion {
        match self.0 {
            InnerHeritageConfig::V1(_) => HeritageConfigVersion::V1,
        }
    }

    /// Returns `true` if this is a V1 HeritageConfig
    pub fn is_v1(&self) -> bool {
        #[allow(unreachable_patterns)]
        match self.0 {
            InnerHeritageConfig::V1(_) => true,
            _ => false,
        }
    }

    /// Returns a reference to the inner V1 HeritageConfig
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidHeritageConfigVersion`] if the inner config is not V1
    pub fn heritage_config_v1(&self) -> Result<&v1::HeritageConfig> {
        #[allow(unreachable_patterns)]
        match &self.0 {
            InnerHeritageConfig::V1(hc) => Ok(hc),
            _ => Err(Error::InvalidHeritageConfigVersion("v1")),
        }
    }

    /// Returns the miniscript expression representing the TapTree for this HeritageConfig
    ///
    /// If an index is provided, it will be used to derive child keys for every extended
    /// public key ([HeirConfig::HeirXPubkey]) in the config.
    ///
    /// Returns `None` only if there are no heirs in this HeritageConfig.
    pub fn descriptor_taptree_miniscript_expression_for_child(
        &self,
        index: Option<u32>,
    ) -> Option<String> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => {
                hc.descriptor_taptree_miniscript_expression_for_child(index)
            }
        }
    }

    /// Returns an iterator over references to the [HeirConfig]s in this HeritageConfig
    ///
    /// For V1 HeritageConfigs, the order is guaranteed to be from the lowest
    /// maturity time to the highest.
    pub fn iter_heir_configs(&self) -> impl Iterator<Item = &HeirConfig> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc.iter_heritages().map(|h| h.get_heir_config()),
        }
    }

    /// Returns a [HeritageExplorer] for the given [HeirConfig] if found
    ///
    /// The HeritageExplorer provides methods to examine the specific heritage
    /// entry and its spending conditions.
    ///
    /// Returns `None` if no matching heritage could be found.
    pub fn get_heritage_explorer(&self, heir_config: &HeirConfig) -> Option<HeritageExplorer> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc
                .get_heritage_explorer(heir_config)
                .map(|he: v1::HeritageExplorer| HeritageExplorer(InnerHeritageExplorer::V1(he))),
        }
    }
}

/// Trait for reconstructing heritage structures from descriptor miniscripts
///
/// This trait provides a way to recover heritage structures (HeritageConfig, Subwallet, etc.)
/// from a descriptor string containing miniscript expressions.
pub trait FromDescriptorScripts {
    /// Reconstructs the structure from descriptor miniscript strings
    ///
    /// # Errors
    ///
    /// Returns an error if the scripts cannot be parsed or don't represent
    /// a valid structure of this type.
    fn from_descriptor_scripts(scripts: &str) -> Result<Self>
    where
        Self: Sized;
}

impl FromDescriptorScripts for HeritageConfig {
    fn from_descriptor_scripts(scripts: &str) -> Result<Self> {
        match v1::HeritageConfig::from_descriptor_scripts(scripts) {
            Ok(hc_v1) => return Ok(HeritageConfig(InnerHeritageConfig::V1(hc_v1))),
            Err(e) => log::info!("{e}"),
        }
        Err(Error::InvalidScriptFragments("any"))
    }
}

/// Trait for exploring characteristics of a specific heir's heritage
///
/// Objects implementing this trait are returned by HeritageConfig and allow
/// examination of heir-specific properties. This is particularly useful
/// during PSBT creation.
pub trait HeritageExplorerTrait {
    /// Returns the index of this heir's miniscript in the TapTree
    ///
    /// This index is used to compute the policy index when creating a PSBT.
    fn get_miniscript_index(&self) -> usize;

    /// Returns the spending conditions for this heritage
    ///
    /// The SpendConditions allow verification of the minimum timestamp
    /// and block height at which the heir can spend a given input.
    fn get_spend_conditions(&self) -> SpendConditions;

    /// Checks if the given [Fingerprint] is part of this heritage
    fn has_fingerprint(&self, fingerprint: Fingerprint) -> bool;

    /// Returns the Bitcoin script corresponding to this heritage for insertion
    /// into the TapScript MAST.
    ///
    /// If origins are provided, they will be used to transform extended public
    /// keys into single public keys.
    ///
    /// # Panics
    ///
    /// Panics if the provided origins don't cover every XPub or are incompatible.
    fn get_script<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> ScriptBuf {
        self.get_miniscript(origins).encode()
    }

    /// Returns the [Miniscript] object of the TapScript section corresponding to this heritage
    ///
    /// If origins are provided, they will be used to transform extended public
    /// keys into single public keys.
    ///
    /// # Panics
    ///
    /// Panics if the provided origins don't cover every XPub or are incompatible.
    fn get_miniscript<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> Miniscript<DefiniteDescriptorKey, Tap> {
        Miniscript::<DefiniteDescriptorKey, Tap>::from_str(&self.get_miniscript_expression(origins))
            .expect("we provide the miniscript so it should be valid")
    }

    /// Returns the [Miniscript] expression string of the TapScript section corresponding to this heritage
    ///
    /// If origins are provided, they will be used to transform extended public
    /// keys into single public keys.
    ///
    /// # Panics
    ///
    /// Panics if the provided origins don't cover every XPub or are incompatible.
    fn get_miniscript_expression<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> String;
}

#[derive(Debug)]
enum InnerHeritageExplorer<'a> {
    V1(v1::HeritageExplorer<'a>),
}

/// Concrete implementation of HeritageExplorerTrait
///
/// This struct wraps version-specific heritage explorer implementations
/// and provides a unified interface for examining heritage properties.
#[derive(Debug)]
pub struct HeritageExplorer<'a>(InnerHeritageExplorer<'a>);
impl<'a> HeritageExplorerTrait for HeritageExplorer<'a> {
    fn get_miniscript_index(&self) -> usize {
        match &self.0 {
            InnerHeritageExplorer::V1(he) => he.get_miniscript_index(),
        }
    }

    fn get_spend_conditions(&self) -> SpendConditions {
        match &self.0 {
            InnerHeritageExplorer::V1(he) => he.get_spend_conditions(),
        }
    }

    fn has_fingerprint(&self, fingerprint: Fingerprint) -> bool {
        match &self.0 {
            InnerHeritageExplorer::V1(he) => he.has_fingerprint(fingerprint),
        }
    }

    fn get_miniscript_expression<'b>(
        &self,
        origins: impl Iterator<Item = (&'b Fingerprint, &'b DerivationPath)>,
    ) -> String {
        match &self.0 {
            InnerHeritageExplorer::V1(he) => he.get_miniscript_expression(origins),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::panic;
    use std::collections::HashSet;

    use crate::tests::get_test_heritage;
    use crate::tests::TestHeritage;

    use super::HeritageConfig;
    use super::InnerHeritageConfig;

    // Just a reminder to extra-check things if the default version is changed in the future
    #[test]
    fn default_heritage_config_is_v1() {
        #[allow(irrefutable_let_patterns)]
        let HeritageConfig(InnerHeritageConfig::V1(_)) = HeritageConfig::builder().build() else {
            panic!("Just a reminder to extra-check things if the default version is changed in the future");
        };
    }

    #[test]
    fn heritage_config_hash_eq() {
        let reference = HeritageConfig::builder_v1()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(90)
            .build();
        // Add order of Heritage(s) does not count
        let same1 = HeritageConfig::builder_v1()
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(90)
            .build();
        // different reference_time
        let different1 = HeritageConfig::builder_v1()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072001)
            .minimum_lock_time(90)
            .build();
        // different minimum_lock_time
        let different2 = HeritageConfig::builder_v1()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(91)
            .build();
        // Different heritage timelock
        let different3 = HeritageConfig::builder_v1()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(
                super::v1::Heritage::new(get_test_heritage(TestHeritage::Wife).heir_config.clone())
                    .time_lock(900),
            )
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(91)
            .build();

        assert_eq!(reference, same1);
        assert_ne!(reference, different1);
        assert_ne!(reference, different2);
        assert_ne!(reference, different3);
        assert_ne!(same1, different1);
        assert_ne!(same1, different2);
        assert_ne!(same1, different3);
        assert_ne!(different1, different2);
        assert_ne!(different1, different3);
        assert_ne!(different2, different3);

        // Hash should follow the EQ/NEQ tests
        let (mut set1, mut set2) = (HashSet::new(), HashSet::new());
        // In set1 we insert everything
        set1.insert(reference.clone());
        set1.insert(same1.clone());
        set1.insert(different1.clone());
        set1.insert(different2.clone());
        set1.insert(different3.clone());
        // In Set2 we do not insert same1
        set2.insert(reference);
        set2.insert(different1);
        set2.insert(different2);
        set2.insert(different3);
        // The two sets should be equals
        assert_eq!(set1, set2);
    }
}
