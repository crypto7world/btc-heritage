use std::{fmt::Debug, str::FromStr};

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

#[derive(Debug, Clone)]
pub struct SpendConditions {
    spendable_timestamp: Option<u64>,
    relative_block_lock: Option<u16>,
}
impl SpendConditions {
    pub fn can_spend_now(&self) -> bool {
        let now = crate::utils::timestamp_now();
        self.can_spend_at(now)
    }

    pub fn can_spend_at(&self, ts: u64) -> bool {
        ts >= self.spendable_timestamp.unwrap_or(0)
    }
    pub fn get_relative_block_lock(&self) -> Option<u16> {
        self.relative_block_lock
    }
    pub fn get_spendable_timestamp(&self) -> Option<u64> {
        self.spendable_timestamp
    }
    pub fn for_owner() -> Self {
        SpendConditions {
            spendable_timestamp: None,
            relative_block_lock: None,
        }
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HeritageConfig(InnerHeritageConfig);

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "version", rename_all = "lowercase")]
enum InnerHeritageConfig {
    V1(v1::HeritageConfig),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeritageConfigVersion {
    V1 = 1,
}

impl HeritageConfig {
    /// Return a builder for the default [HeritageConfig] version
    pub fn builder() -> v1::HeritageConfigBuilder {
        HeritageConfig::builder_v1()
    }

    /// Return a builder for [HeritageConfig::V1]
    pub fn builder_v1() -> v1::HeritageConfigBuilder {
        v1::HeritageConfig::builder()
    }

    /// Return the version
    pub fn version(&self) -> HeritageConfigVersion {
        match self.0 {
            InnerHeritageConfig::V1(_) => HeritageConfigVersion::V1,
        }
    }

    /// Return `true` if this is an [HeritageConfig::V1]
    pub fn is_v1(&self) -> bool {
        #[allow(unreachable_patterns)]
        match self.0 {
            InnerHeritageConfig::V1(_) => true,
            _ => false,
        }
    }

    /// Borrow the specific, inner, [v1::HeritageConfig] encapsulated in this [HeritageConfig]
    ///
    /// # Errors
    /// Return an error if the inner object is not V1
    pub fn heritage_config_v1(&self) -> Result<&v1::HeritageConfig> {
        #[allow(unreachable_patterns)]
        match &self.0 {
            InnerHeritageConfig::V1(hc) => Ok(hc),
            _ => Err(Error::InvalidHeritageConfigVersion("v1")),
        }
    }

    /// Returns the miniscript expression representing the TapTree generated
    /// by this [HeritageConfig], if any.
    /// If present, the index will be used to derive a child for every xpub present in this [HeritageConfig],
    /// i.e. for every [HeirConfig::HeirXPubkey]. For other HeirConfig, it has no effect.
    /// The only case where this returns [None] is if there is no heir in this [HeritageConfig].
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

    /// Returns an iterator over references to the [HeirConfig]s present in the [HeritageConfig].
    /// No particular order is guaranteed.
    pub fn iter_heir_configs(&self) -> impl Iterator<Item = &HeirConfig> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc.iter_heritages().map(|h| h.get_heir_config()),
        }
    }

    /// Returns a type with [HeritageExplorer] for the given [HeirConfig] if it can be found in the [HeritageConfig].
    /// If no Heritage could be matched, the function returns [None].
    pub fn get_heritage_explorer(&self, heir_config: &HeirConfig) -> Option<HeritageExplorer> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc
                .get_heritage_explorer(heir_config)
                .map(|he: v1::HeritageExplorer| HeritageExplorer(InnerHeritageExplorer::V1(he))),
        }
    }
}

/// This trait is for objects that can be returned by an [HeritageConfig] and
/// allow to explore caracteritics of a specific Heir. Usefull at the
/// PSBT creation stage
pub trait HeritageExplorerTrait {
    /// Indicate the index of this Heir miniscript in the miniscript TapTree of the
    /// [HeritageConfig]. Used to compute the policy index when creating a PSBT
    fn get_miniscript_index(&self) -> usize;

    /// Retrieve the [SpendConditions] for the Heritage, allowing
    /// to verify the minimum timestamp and block height at which
    /// they can spend a given input
    fn get_spend_conditions(&self) -> SpendConditions;

    /// Verify if the given [Fingerprint] is part of the Heritage being explored
    fn has_fingerprint(&self, fingerprint: Fingerprint) -> bool;

    /// Get the actual Bitcoin lock script for the Heritage in this [HeritageConfig].
    /// If `origins` are provided, they will be used to transform eXtended Public Keys into
    /// Single Public Keys.
    ///
    /// # Panics
    /// Panics if the provided `origins` were not covering every XPub or were not
    /// compatible
    fn get_script<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> ScriptBuf {
        self.get_miniscript(origins).encode()
    }

    /// Get the [Miniscript] object for the Heritage in this [HeritageConfig].
    /// If `origins` are provided, they will be used to transform eXtended Public Keys into
    /// Single Public Keys.
    ///
    /// # Panics
    /// Panics if the provided `origins` were not covering every XPub or were not
    /// compatible
    fn get_miniscript<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> Miniscript<DefiniteDescriptorKey, Tap> {
        Miniscript::<DefiniteDescriptorKey, Tap>::from_str(&self.get_miniscript_expression(origins))
            .expect("we provide the miniscript so it should be valid")
    }

    /// Get the miniscript expression for the Heritage in this [HeritageConfig].
    /// If `origins` are provided, they will be used to transform eXtended Public Keys into
    /// Single Public Keys.
    ///
    /// # Panics
    /// Panics if the provided `origins` were not covering every XPub or were not
    /// compatible
    fn get_miniscript_expression<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> String;
}

#[derive(Debug)]
enum InnerHeritageExplorer<'a> {
    V1(v1::HeritageExplorer<'a>),
}

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
            panic!();
        };
    }

    // Just a reminder to extra-check things if the default version is changed in the future
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
