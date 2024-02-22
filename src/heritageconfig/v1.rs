use std::collections::HashSet;

use bdk::bitcoin::absolute::LOCK_TIME_THRESHOLD;
use serde::{Deserialize, Serialize};

use super::{heirtypes::HeirConfig, SpendConditions};

const SEC_IN_A_DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Hash, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Days(u16);
impl Default for Days {
    fn default() -> Self {
        Self(365)
    }
}
impl std::ops::Add<Self> for Days {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Days(self.0 + rhs.0)
    }
}

macro_rules! days_mul_impl {
    ($t:ty) => {
        impl std::ops::Mul<$t> for Days {
            type Output = Self;
            fn mul(self, rhs: $t) -> Self::Output {
                Self(
                    self.0
                        .checked_mul(u16::try_from(rhs).unwrap_or(u16::MAX))
                        .unwrap_or(u16::MAX),
                )
            }
        }
    };
}
days_mul_impl!(u8);
days_mul_impl!(u16);
days_mul_impl!(u32);
days_mul_impl!(u64);
days_mul_impl!(usize);

impl Days {
    pub fn as_seconds(self) -> u64 {
        self.0 as u64 * SEC_IN_A_DAY
    }

    pub fn as_u16(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct Heritage {
    pub heir_config: HeirConfig,
    // For this heritage, how many days from the reference time of the HeritageConfig?
    pub time_lock: Days,
}

impl PartialEq for Heritage {
    fn eq(&self, other: &Self) -> bool {
        self.heir_config == other.heir_config && self.time_lock.0 == other.time_lock.0
    }
}
impl PartialOrd for Heritage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Heritage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.time_lock.0.cmp(&other.time_lock.0) {
            std::cmp::Ordering::Equal => self.heir_config.cmp(&other.heir_config),
            other => other,
        }
    }
}

impl Eq for Heritage {}

impl Heritage {
    pub fn new(heir_config: HeirConfig) -> Self {
        Self {
            heir_config,
            time_lock: Days::default(),
        }
    }

    pub fn time_lock(mut self, time_lock: u16) -> Self {
        self.time_lock = Days(time_lock);
        self
    }

    fn time_lock_in_seconds(&self) -> u64 {
        self.time_lock.as_seconds()
    }

    pub fn get_heir_config(&self) -> &HeirConfig {
        &self.heir_config
    }
}

#[derive(Debug, Clone, Hash, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct ReferenceTimestamp(u64);
impl Default for ReferenceTimestamp {
    fn default() -> Self {
        // Compute the reference_timestamp by taking the current timestamp and rounding it to today at noon
        // In effect, this is "Today at 12:00 (24H) UTC"
        let current_time = crate::utils::timestamp_now();
        let distance_from_midnight = current_time % SEC_IN_A_DAY;
        let reference_timestamp = current_time - distance_from_midnight + SEC_IN_A_DAY / 2;

        Self(reference_timestamp)
    }
}
impl ReferenceTimestamp {
    pub fn as_u64(&self) -> u64 {
        self.0
    }
}
#[derive(Debug, Clone, Hash, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct MinimumLockTime(Days);
impl Default for MinimumLockTime {
    fn default() -> Self {
        Self(Days(30))
    }
}
impl std::ops::Add<Self> for MinimumLockTime {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        MinimumLockTime(self.0 + rhs.0)
    }
}
impl std::ops::Add<Days> for MinimumLockTime {
    type Output = Self;

    fn add(self, rhs: Days) -> Self::Output {
        MinimumLockTime(Days(self.0 .0 + rhs.0))
    }
}
impl<T> std::ops::Mul<T> for MinimumLockTime
where
    Days: std::ops::Mul<T, Output = Days>,
{
    type Output = Self;
    fn mul(self, rhs: T) -> Self::Output {
        Self(self.0 * rhs)
    }
}
impl MinimumLockTime {
    pub fn as_blocks(&self) -> u16 {
        // One block every 10min on average
        // 24 hours in a day, 6 blocks per hour
        u16::try_from(self.0 .0 * 24 * 6).unwrap_or(u16::MAX)
    }

    pub fn as_days(&self) -> &Days {
        &self.0
    }
}
// There are only two ways of creating this Struct:
//  - through the HeritageConfigBuilder -> it will create a sorted Vec
//  - through Deserializing -> the custom Deserializer ensure the Vec is sorted
#[derive(Debug, Clone, Hash, Serialize, PartialEq, Eq)]
#[serde(transparent)]
struct Heritages(Vec<Heritage>);
impl<'de> Deserialize<'de> for Heritages {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut v = Deserialize::deserialize(deserializer).map(Heritages)?;
        v.normalize();
        Ok(v)
    }
}

impl Heritages {
    /// Ensure that the [HeritageModes] vector is sorted and "deduplicated"
    /// - no two [Heritage] can have the same lock_time
    /// - no two [Heritage] can have the same [HeirConfig]
    fn normalize(&mut self) {
        // First sort by (lock_time, mode)
        // It ensures that the same content is always processed the same way
        // 2 Heritage are equals when locktime and pub key are equals (contacts are irrelevant)
        self.0.sort();

        // Then dedup HeirConfig using a HashSet
        let mut seen = HashSet::new();
        self.0.retain(|e| {
            if !seen.contains(&e.heir_config) {
                seen.insert(e.heir_config.clone());
                return true;
            }
            false
        });

        // Finaly, dedup locktimes
        self.0.dedup_by_key(|e| e.time_lock.0);
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
pub struct HeritageConfig {
    /// The deduplicated, ordered list of [Heritage] for this [HeritageConfig].
    heritages: Heritages,
    /// This is the reference timestamp that is used to compute the LockTime(s)
    #[serde(default)]
    pub reference_timestamp: ReferenceTimestamp,
    /// This is the number of days we want to enforce before an heir can consumme an input.
    /// It exist in case an old address with an old absolute locktime is used
    #[serde(default)]
    pub minimum_lock_time: MinimumLockTime,
}

impl HeritageConfig {
    pub fn heritages(&self) -> &Vec<Heritage> {
        &self.heritages.0
    }

    pub(crate) fn builder() -> HeritageConfigBuilder {
        HeritageConfigBuilder::default()
    }
    pub fn descriptor_taptree_miniscript_expression(&self) -> Option<String> {
        if self.heritages.0.len() == 0 {
            return None;
        }

        // Create a vector of sorted Miniscript conditions
        // sorted by lockTime ascending (because of the Heritage sorting)
        let sorted_conditions: Vec<String> = (0..self.heritages.0.len())
            .map(|idx| self.get_heritage_script_string(idx))
            .collect();

        // The tree construction strategy is such that the first Heir always have the minimum tree depth
        // The more we go further down in Heirs, the more depth
        // For example, if there are 5 heirs, here is the conditions-tree:
        //               root
        //               /  \
        //              .    .
        //              H1  / \
        //                 .   .
        //                 H2 / \
        //                   .   .
        //                   H3 / \
        //                     H4 H5
        // It means that the further the heir in the succession order, the more they will have to pay in TX fee to retrieve the funds

        sorted_conditions
            .into_iter()
            .rev()
            .fold(None, |acc, condition| {
                Some(match acc {
                    Some(acc) => format!("{{{condition},{acc}}}"),
                    None => condition,
                })
            })
    }

    fn get_heritage_spend_condition(&self, heritage_index: usize) -> SpendConditions {
        // Private method, we control the index and know it's valid
        let heritage = &self.heritages.0[heritage_index];
        SpendConditions {
            spendable_timestamp: Some(self.reference_timestamp.0 + heritage.time_lock_in_seconds()),
            relative_block_lock: Some((self.minimum_lock_time * (heritage_index + 1)).as_blocks()),
        }
    }
    fn get_heritage_script_string(&self, heritage_index: usize) -> String {
        // Private method, we control the index and know it's valid
        let heritage = &self.heritages.0[heritage_index];
        let SpendConditions {
            spendable_timestamp: Some(absolute_lock_time),
            relative_block_lock: Some(rel_lock_time),
        } = self.get_heritage_spend_condition(heritage_index)
        else {
            unreachable!("In this version of the software, there is always Some(...) values for an Heir in the SpendConditionTester");
        };
        // No matter what, this should always be greater than LOCK_TIME_THRESHOLD (500_000_000)
        assert!(absolute_lock_time > LOCK_TIME_THRESHOLD as u64, "absolute_lock_time cannot be less or equal to {LOCK_TIME_THRESHOLD} because it changes its meaning");
        // No matter what, this should always be > 1440 blocks = 10 days
        assert!(
            rel_lock_time >= 1440,
            "rel_lock_time cannot be less than 1440 as a safety mesure"
        );

        let heritage_fragment = heritage.heir_config.descriptor_segment();
        format!("and_v({heritage_fragment},and_v(v:older({rel_lock_time}),after({absolute_lock_time})))")
    }
    pub(crate) fn get_heritage_explorer(
        &self,
        heir_config: &HeirConfig,
    ) -> Option<HeritageExplorer> {
        let index = self
            .heritages
            .0
            .iter()
            .position(|e| e.get_heir_config() == heir_config);

        index.map(|index| HeritageExplorer {
            heritage_config: self,
            heritage_index: index,
            miniscript_expression: self.get_heritage_script_string(index),
        })
    }
}

#[derive(Debug, Default)]
pub struct HeritageConfigBuilder {
    heritages: Vec<Heritage>,
    // This is the reference timestamp that is used to compute the LockTime(s)
    reference_timestamp: ReferenceTimestamp,
    // This is the number of days we want to enforce before an heir can consumme an input
    // It exist in case an old address with an old absolute locktime is used
    minimum_lock_time: MinimumLockTime,
}

impl HeritageConfigBuilder {
    pub fn add_heritage(mut self, heritage: Heritage) -> Self {
        self.heritages.push(heritage);
        self
    }
    pub fn expand_heritages(mut self, heritages: impl IntoIterator<Item = Heritage>) -> Self {
        self.heritages
            .append(&mut Vec::from_iter(heritages.into_iter()));
        self
    }
    pub fn reference_time(mut self, reference_time: u64) -> Self {
        self.reference_timestamp = ReferenceTimestamp(reference_time);
        self
    }
    pub fn minimum_lock_time(mut self, minimum_lock_time: u16) -> Self {
        self.minimum_lock_time = MinimumLockTime(Days(minimum_lock_time));
        self
    }
    pub fn build(self) -> super::HeritageConfig {
        // Create Heritages from the Vec of Heritage and normalize it
        let mut heritages = Heritages(self.heritages);
        heritages.normalize();
        super::HeritageConfig(super::InnerHeritageConfig::V1(HeritageConfig {
            heritages,
            reference_timestamp: self.reference_timestamp,
            minimum_lock_time: self.minimum_lock_time,
        }))
    }
}

#[derive(Debug)]
pub(crate) struct HeritageExplorer<'a> {
    heritage_config: &'a HeritageConfig,
    heritage_index: usize,
    miniscript_expression: String,
}

impl<'a> super::HeritageExplorerTrait for HeritageExplorer<'a> {
    fn get_miniscript_index(&self) -> usize {
        self.heritage_index
    }

    fn get_spend_conditions(&self) -> SpendConditions {
        let heritage = &self.heritage_config.heritages.0[self.heritage_index];
        SpendConditions {
            spendable_timestamp: Some(
                self.heritage_config.reference_timestamp.0 + heritage.time_lock_in_seconds(),
            ),
            relative_block_lock: Some(
                (self.heritage_config.minimum_lock_time * (self.heritage_index + 1)).as_blocks(),
            ),
        }
    }

    fn get_miniscript_expression(&self) -> &str {
        &self.miniscript_expression
    }
}

#[cfg(test)]
#[allow(irrefutable_let_patterns)]
mod tests {

    use crate::tests::*;

    use super::super::HeritageConfig as VHeritageConfig;
    use super::super::InnerHeritageConfig as IHC;
    use super::HeritageConfig as HeritageConfigV1;

    #[test]
    fn heritage_config_always_sorted() {
        // Wife (lock_time=90) should alays come before Brother (lock_time=180)
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(90);
        let h2 = get_test_heritage(TestHeritage::Brother).time_lock(180);

        let VHeritageConfig(IHC::V1(hc1)) = HeritageConfigV1::builder()
            .add_heritage(h1.clone())
            .add_heritage(h2.clone())
            .build()
        else {
            unreachable!("we asked for v1")
        };

        let VHeritageConfig(IHC::V1(hc2)) = HeritageConfigV1::builder()
            .add_heritage(h2.clone())
            .add_heritage(h1.clone())
            .build()
        else {
            unreachable!("we asked for v1")
        };
        let VHeritageConfig(IHC::V1(hc3)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h2).unwrap(),
            serde_json::to_string(&h1).unwrap(),
        ))
        .unwrap();
        assert_eq!(hc1.heritages.0, hc2.heritages.0);
        assert_eq!(hc2.heritages.0, hc3.heritages.0);
        assert_eq!(hc1.heritages.0, hc3.heritages.0);
    }

    #[test]
    fn cannot_have_same_pubkey_twice() {
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(180);
        let h2 = get_test_heritage(TestHeritage::Brother).time_lock(365);
        let h3 = get_test_heritage(TestHeritage::Brother).time_lock(720);
        let h4 = get_test_heritage(TestHeritage::Wife).time_lock(720);
        let VHeritageConfig(IHC::V1(hc1)) = HeritageConfigV1::builder()
            .add_heritage(h1.clone())
            .add_heritage(h2.clone())
            .build()
        else {
            unreachable!("we asked for v1")
        };
        let VHeritageConfig(IHC::V1(hc2)) = HeritageConfigV1::builder()
            .add_heritage(h1.clone())
            .add_heritage(h2.clone())
            .add_heritage(h3.clone())
            .build()
        else {
            unreachable!("we asked for v1")
        };
        let VHeritageConfig(IHC::V1(hc3)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h2).unwrap(),
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h4).unwrap(),
        ))
        .unwrap();
        assert_eq!(hc1.heritages.0, hc2.heritages.0);
        assert_eq!(hc2.heritages.0, hc3.heritages.0);
        assert_eq!(hc1.heritages.0, hc3.heritages.0);
    }

    #[test]
    fn cannot_have_same_locktime_twice() {
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(180);
        let h2 = get_test_heritage(TestHeritage::Brother).time_lock(180);

        let VHeritageConfig(IHC::V1(hc1)) =
            HeritageConfigV1::builder().add_heritage(h2.clone()).build()
        else {
            unreachable!("we asked for v1")
        };
        let VHeritageConfig(IHC::V1(hc2)) = HeritageConfigV1::builder()
            .add_heritage(h1.clone())
            .add_heritage(h2.clone())
            .build()
        else {
            unreachable!("we asked for v1")
        };
        let VHeritageConfig(IHC::V1(hc3)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h2).unwrap(),
            serde_json::to_string(&h1).unwrap(),
        ))
        .unwrap();
        assert_eq!(hc1.heritages.0, hc2.heritages.0);
        assert_eq!(hc2.heritages.0, hc3.heritages.0);
        assert_eq!(hc1.heritages.0, hc3.heritages.0);
    }

    #[test]
    fn hc_equalities() {
        let h1 = get_test_heritage(TestHeritage::Wife);
        let h2 = get_test_heritage(TestHeritage::Brother);
        let VHeritageConfig(IHC::V1(hc1)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        let VHeritageConfig(IHC::V1(hc2)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h2).unwrap(),
            serde_json::to_string(&h1).unwrap()
        ))
        .unwrap();
        assert_eq!(hc1, hc2);
        assert_eq!(
            hc1.descriptor_taptree_miniscript_expression().unwrap(),
            hc2.descriptor_taptree_miniscript_expression().unwrap()
        );
    }

    #[test]
    fn hc_ne_with_ne_hlocktime() {
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(180);
        let h2 = get_test_heritage(TestHeritage::Brother).time_lock(360);
        let VHeritageConfig(IHC::V1(hc1)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(181);
        let h2 = get_test_heritage(TestHeritage::Brother).time_lock(360);
        let VHeritageConfig(IHC::V1(hc2)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();

        assert_ne!(hc1, hc2);
        assert_ne!(
            hc1.descriptor_taptree_miniscript_expression().unwrap(),
            hc2.descriptor_taptree_miniscript_expression().unwrap()
        );
    }

    #[test]
    fn hc_ne_with_ne_minlocktime() {
        let h1 = get_test_heritage(TestHeritage::Wife);
        let h2 = get_test_heritage(TestHeritage::Brother);
        let VHeritageConfig(IHC::V1(hc1)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":60,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        let VHeritageConfig(IHC::V1(hc2)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        assert_ne!(hc1, hc2);
        assert_ne!(
            hc1.descriptor_taptree_miniscript_expression().unwrap(),
            hc2.descriptor_taptree_miniscript_expression().unwrap()
        );
    }

    #[test]
    fn hc_ne_with_ne_reftimestamp() {
        let h1 = get_test_heritage(TestHeritage::Wife);
        let h2 = get_test_heritage(TestHeritage::Brother);
        let VHeritageConfig(IHC::V1(hc1)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900800
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        let VHeritageConfig(IHC::V1(hc2)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":1702900900
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap()
        ))
        .unwrap();
        assert_ne!(hc1, hc2);
        assert_ne!(
            hc1.descriptor_taptree_miniscript_expression().unwrap(),
            hc2.descriptor_taptree_miniscript_expression().unwrap()
        );
    }

    #[test]
    fn heritage_config_expected_miniscript() {
        let h1 = get_test_heritage(TestHeritage::Wife).time_lock(360);
        let h2 = get_test_heritage(TestHeritage::Backup).time_lock(180);
        let h3 = get_test_heritage(TestHeritage::Brother).time_lock(720);
        let h4 = get_test_heritage(TestHeritage::Brother).time_lock(180);
        let VHeritageConfig(IHC::V1(hc)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {},
                    {},
                    {},
                    {}
                ],
                "minimum_lock_time":30,"reference_timestamp":1700000000
            }}"#,
            serde_json::to_string(&h1).unwrap(),
            serde_json::to_string(&h2).unwrap(),
            serde_json::to_string(&h3).unwrap(),
            serde_json::to_string(&h4).unwrap()
        ))
        .unwrap();
        // Compared to the JSON HeritageConfig, the following happens:
        // - Brother/720 is discarded because Brother/180 is already there, we cannot have the same pubkey twice and 180 comes before 720
        // - Backup/180 is discarded because Brother/180 is already there, we cannot have the same locktime twice and Backup pubkey comes after Brother pubkey in lexical order
        // - Therefore, after sorting and dedup, there are 2 Heritages: Brother/180 and Wife/360 respectively.
        // minimum_lock_time is 30 days, so 4320 blocks, will be used for the miniscript of Brother as the "older" condition
        // reference timestamp is 1700000000, 1700000000 + 180*24*3600 = 1715552000, will be used for the "after" condition
        // for the miniscript of Wife, it is the 2nd heir so the "older" condition is twice the min_lock_time: 8640 blocks
        //reference timestamp is 1700000000, 1700000000 + 360*24*3600 = 1731104000, will be used for the "after" condition

        let backup_pubkey = get_test_heir_pubkey(TestHeritage::Backup);
        let wife_pubkey = get_test_heir_pubkey(TestHeritage::Wife);
        let brother_pubkey = get_test_heir_pubkey(TestHeritage::Brother);
        // Control the lexicographical order of keys, because it is important for this test
        // In case we change the test pubkeys someday and this test fails, this should hint
        // weither the HeritageConfig logic broke or we just happened to choose "bad" pub keys
        assert!(brother_pubkey < wife_pubkey);
        assert!(wife_pubkey < backup_pubkey);

        let expected_descriptor_fragment = format!(
            "{{\
            and_v(v:pk({brother_pubkey}),and_v(v:older(4320),after(1715552000))),\
            and_v(v:pk({wife_pubkey}),and_v(v:older(8640),after(1731104000)))\
            }}"
        );
        assert_eq!(
            expected_descriptor_fragment,
            hc.descriptor_taptree_miniscript_expression().unwrap()
        );

        assert_eq!(
            get_test_heritage_config(TestHeritageConfig::BackupWifeBro)
                .descriptor_taptree_miniscript_expression()
                .unwrap(),
            format!(
                "{{and_v(v:pk({backup_pubkey}),and_v(v:older(12960),after(1794608000))),\
                {{and_v(v:pk({wife_pubkey}),and_v(v:older(25920),after(1797632000))),\
                and_v(v:pk({brother_pubkey}),and_v(v:older(38880),after(1800656000)))}}}}"
            )
        );
    }
}
