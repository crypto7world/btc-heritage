use std::{collections::HashSet, fmt::Debug, str::FromStr};

use bdk::{
    bitcoin::{secp256k1::XOnlyPublicKey, ScriptBuf},
    miniscript::{DefiniteDescriptorKey, Miniscript, Tap, ToPublicKey},
};
use serde::{Deserialize, Serialize};

use self::heirtypes::HeirConfig;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HeritageConfig(InnerHeritageConfig);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "version", rename_all = "lowercase")]
enum InnerHeritageConfig {
    V1(v1::HeritageConfig),
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

    /// Returns the miniscript expression representing the TapTree generated
    /// by this [HeritageConfig], if any. The only case where this is [None] is
    /// if there is no heir in this [HeritageConfig].
    pub fn descriptor_taptree_miniscript_expression(&self) -> Option<String> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc.descriptor_taptree_miniscript_expression(),
        }
    }

    /// Returns a type with [HeritageExplorer] for the given [HeirConfig] if it can be found in the [HeritageConfig].
    /// If no Heritage could be matched, the function returns [None].
    pub fn get_heritage_explorer(&self, heir_config: &HeirConfig) -> Option<HeritageExplorer> {
        match &self.0 {
            InnerHeritageConfig::V1(hc) => hc
                .get_heritage_explorer(heir_config)
                .map(|he| HeritageExplorer(InnerHeritageExplorer::V1(he))),
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

    /// Get the actual Bitcoin lock script for the Heritage in this [HeritageConfig].
    fn get_script(&self) -> ScriptBuf {
        self.get_miniscript().encode()
    }

    /// Get a [HashSet] of the [XOnlyPublicKey] for the Heritage in this [HeritageConfig].
    fn get_xpubkeys_set(&self) -> HashSet<XOnlyPublicKey> {
        self.get_miniscript()
            .iter_pk()
            .map(|dpk| dpk.to_public_key().inner.to_x_only_pubkey())
            .collect::<HashSet<_>>()
    }

    /// Get the [Miniscript] object for the Heritage in this [HeritageConfig].
    fn get_miniscript(&self) -> Miniscript<DefiniteDescriptorKey, Tap> {
        Miniscript::<DefiniteDescriptorKey, Tap>::from_str(self.get_miniscript_expression())
            .expect("we provide the miniscript so it should be valid")
    }

    /// Get the miniscript expression for the Heritage in this [HeritageConfig].
    fn get_miniscript_expression(&self) -> &str;
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

    fn get_miniscript_expression(&self) -> &str {
        match &self.0 {
            InnerHeritageExplorer::V1(he) => he.get_miniscript_expression(),
        }
    }
}

#[cfg(test)]
mod tests {
    use core::panic;

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
}
