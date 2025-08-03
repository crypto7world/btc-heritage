use std::{collections::HashSet, str::FromStr};

use serde::{Deserialize, Serialize};

use super::{heirtypes::HeirConfig, SpendConditions};
use crate::{
    bitcoin::{
        absolute::LOCK_TIME_THRESHOLD,
        bip32::{DerivationPath, Fingerprint},
    },
    errors::Error,
};

const SEC_IN_A_DAY: u64 = 24 * 60 * 60;

// One block every 10min on average
// 24 hours in a day, 6 blocks per hour
const BLOCKS_IN_A_DAY: u16 = 24 * 6;

#[derive(Debug, Clone, Hash, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct Days(u16);
impl Default for Days {
    fn default() -> Self {
        Self(365)
    }
}
impl core::ops::Add<Self> for Days {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Days(self.0 + rhs.0)
    }
}
impl FromStr for Days {
    type Err = <u16 as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Days(s.parse::<u16>()?))
    }
}

macro_rules! days_mul_impl {
    ($t:ty) => {
        impl core::ops::Mul<$t> for Days {
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
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Heritage {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        match self.time_lock.0.cmp(&other.time_lock.0) {
            core::cmp::Ordering::Equal => self.heir_config.cmp(&other.heir_config),
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

mod reference_timestamp {
    use std::marker::PhantomData;

    use super::{Deserialize, Serialize, LOCK_TIME_THRESHOLD, SEC_IN_A_DAY};

    #[derive(Debug, Clone, Hash, Copy, Serialize, PartialEq, Eq)]
    #[serde(transparent)]
    pub struct ReferenceTimestamp(pub(super) u64, PhantomData<()>);
    impl Default for ReferenceTimestamp {
        fn default() -> Self {
            // Compute the reference_timestamp by taking the current timestamp and rounding it to today at noon
            // In effect, this is "Today at 12:00 (24H) UTC"
            let current_time = crate::utils::timestamp_now();
            let distance_from_midnight = current_time % SEC_IN_A_DAY;
            let reference_timestamp = current_time - distance_from_midnight + SEC_IN_A_DAY / 2;

            Self::new(reference_timestamp)
        }
    }
    impl ReferenceTimestamp {
        pub fn new(reference_time: u64) -> Self {
            // No matter what, this should always be greater than LOCK_TIME_THRESHOLD (500_000_000)
            assert!(reference_time > LOCK_TIME_THRESHOLD as u64, "reference_time cannot be less or equal to {LOCK_TIME_THRESHOLD} because it would change the meaning of absolute_lock_time");
            Self(reference_time, PhantomData)
        }
        pub fn as_u64(&self) -> u64 {
            self.0
        }
    }
    impl<'de> Deserialize<'de> for ReferenceTimestamp {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Ok(ReferenceTimestamp::new(u64::deserialize(deserializer)?))
        }
    }
}
pub use reference_timestamp::ReferenceTimestamp;

mod minimum_lock_time {
    use std::marker::PhantomData;

    use super::{Days, Deserialize, Serialize, BLOCKS_IN_A_DAY};
    use crate::{bitcoin::Network, utils::bitcoin_network};

    #[derive(Debug, Clone, Hash, Copy, Serialize, PartialEq, Eq)]
    #[serde(transparent)]
    pub struct MinimumLockTime(pub(super) Days, PhantomData<()>);
    impl Default for MinimumLockTime {
        fn default() -> Self {
            Self::new(Days(30))
        }
    }
    impl From<Days> for MinimumLockTime {
        fn from(value: Days) -> Self {
            MinimumLockTime::new(value)
        }
    }
    impl core::ops::Add<Self> for MinimumLockTime {
        type Output = Self;

        fn add(self, rhs: Self) -> Self::Output {
            MinimumLockTime::new(self.0 + rhs.0)
        }
    }
    impl core::ops::Add<Days> for MinimumLockTime {
        type Output = Self;

        fn add(self, rhs: Days) -> Self::Output {
            MinimumLockTime::new(Days(self.0 .0 + rhs.0))
        }
    }
    impl<T> core::ops::Mul<T> for MinimumLockTime
    where
        Days: core::ops::Mul<T, Output = Days>,
    {
        type Output = Self;
        fn mul(self, rhs: T) -> Self::Output {
            Self::new(self.0 * rhs)
        }
    }
    impl MinimumLockTime {
        pub fn new(minimum_lock_time: Days) -> Self {
            // No matter what, this should always be > 10 days in production
            if bitcoin_network::get() == Network::Bitcoin {
                assert!(
                    minimum_lock_time >= Days(10),
                    "minimum_lock_time cannot be less than 10 days as a safety mesure"
                );
            } else {
                // Else simply ensure it is present, with a minimum of 1 day
                assert!(
                    minimum_lock_time >= Days(1),
                    "minimum_lock_time cannot be less than 1 day as a safety mesure"
                );
            }
            Self(minimum_lock_time, PhantomData)
        }
        pub fn as_blocks(&self) -> u16 {
            u16::try_from(self.0 .0 * BLOCKS_IN_A_DAY).unwrap_or(u16::MAX)
        }

        pub fn as_days(&self) -> &Days {
            &self.0
        }
    }
    impl<'de> Deserialize<'de> for MinimumLockTime {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            Ok(MinimumLockTime::new(Days(u16::deserialize(deserializer)?)))
        }
    }
}
pub use minimum_lock_time::MinimumLockTime;

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
    pub fn iter_heritages(&self) -> impl Iterator<Item = &Heritage> {
        self.heritages.0.iter()
    }

    #[deprecated(since = "0.9.0", note = "Prefer using iter_heritages instead")]
    pub fn heritages(&self) -> &Vec<Heritage> {
        &self.heritages.0
    }

    pub(crate) fn builder() -> HeritageConfigBuilder {
        HeritageConfigBuilder::default()
    }
    pub fn descriptor_taptree_miniscript_expression_for_child(
        &self,
        index: Option<u32>,
    ) -> Option<String> {
        if self.heritages.0.len() == 0 {
            return None;
        }

        // Create a vector of sorted Miniscript conditions
        // sorted by lockTime ascending (because of the Heritage sorting)
        let sorted_conditions: Vec<String> = (0..self.heritages.0.len())
            .map(|idx| self.get_heritage_script_string(idx, index))
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

    fn get_lock_times(&self, heritage_index: usize) -> (u16, u64) {
        let SpendConditions {
            spendable_timestamp: Some(absolute_lock_time),
            relative_block_lock: Some(rel_lock_time),
        } = self.get_heritage_spend_condition(heritage_index)
        else {
            unreachable!("In this version of the software, there is always Some(...) values for an Heir in the SpendConditionTester");
        };

        (rel_lock_time, absolute_lock_time)
    }

    fn get_heritage_script_string(
        &self,
        heritage_index: usize,
        xpub_child_index: Option<u32>,
    ) -> String {
        // Private method, we control the index and know it's valid
        let heritage = &self.heritages.0[heritage_index];
        let (rel_lock_time, absolute_lock_time) = self.get_lock_times(heritage_index);
        let heritage_fragment = heritage.heir_config.descriptor_segment(xpub_child_index);
        format!("and_v({heritage_fragment},and_v(v:older({rel_lock_time}),after({absolute_lock_time})))")
    }

    fn get_concrete_heritage_script_string<'a>(
        &self,
        heritage_index: usize,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> String {
        // Private method, we control the index and know it's valid
        let heritage = &self.heritages.0[heritage_index];
        let (rel_lock_time, absolute_lock_time) = self.get_lock_times(heritage_index);
        let concrete_heritage_fragment = heritage.heir_config.concrete_script_segment(origins);
        format!("and_v({concrete_heritage_fragment},and_v(v:older({rel_lock_time}),after({absolute_lock_time})))")
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
        })
    }
}

fn fragment_scripts(scripts: &str) -> Vec<&str> {
    let mut res = Vec::new();

    let mut inception_lvl = 0u32;
    let mut star_index = None;
    let mut start_inception_stack = vec![];
    for (index, char) in scripts.char_indices() {
        match char {
            '{' => {
                // Open braket means we expect 2 scripts fragment separated by a comma
                // Potentialy begining at the following index
                // Mark the begining of the recording
                star_index = Some(index + 1);
                // Increment the inception level
                inception_lvl += 1;
                // Mark that we expect the comma and closing braket at that inception_lvl
                start_inception_stack.push(inception_lvl);
            }
            ',' => {
                // If the comma is at the currently expected inception_lvl for the script separation
                if start_inception_stack
                    .last()
                    .is_some_and(|lvl| *lvl == inception_lvl)
                {
                    // If there is currently a start_index, indicating that we are indeed recording a fragment
                    if let Some(star_index) = star_index {
                        // Then push it
                        let s = &scripts[star_index..index];
                        res.push(s);
                    }
                    // In any case, mark the begining of the recording of the second part
                    star_index = Some(index + 1);
                }
            }
            '}' => {
                // If the closing braket is at the currently expected inception_lvl for a script end
                if start_inception_stack
                    .last()
                    .is_some_and(|lvl| *lvl == inception_lvl)
                {
                    // If there is currently a start_index, indicating that we are indeed recording a fragment
                    if let Some(star_index) = star_index {
                        let s = &scripts[star_index..index];
                        res.push(s);
                    }
                    // In any case, end recording
                    star_index = None;
                }
                // In any case, decrease inception_lvl
                inception_lvl -= 1;
                // We finished processing this inception_lvl, pop it off the stack
                start_inception_stack.pop();
            }
            '(' => inception_lvl += 1,
            ')' => inception_lvl -= 1,
            _ => (),
        }
    }
    assert_eq!(inception_lvl, 0);
    if res.len() == 0 && scripts.len() != 0 {
        res.push(scripts)
    }
    res
}
/// Extract the component of an Heritage v1 script fragment
fn re_v1_fragment() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"^and_v\((?<heir>.+?),and_v\(v:older\((?<rlock>[0-9]+?)\),after\((?<alock>[0-9]+?)\)\)\)$").unwrap())
}

impl super::FromDescriptorScripts for HeritageConfig {
    fn from_descriptor_scripts(scripts: &str) -> crate::errors::Result<Self> {
        let script_fragments = fragment_scripts(scripts);
        if script_fragments.len() == 0 {
            return Ok(HeritageConfigBuilder::default().build_v1());
        }
        let mut heritage_parts = script_fragments
            .into_iter()
            .map(|fragment| {
                let caps = re_v1_fragment().captures(fragment).ok_or_else(|| {
                    log::info!("Failed to match fragment: {fragment}");
                    Error::InvalidScriptFragments("v1")
                })?;
                let heir_config = HeirConfig::from_descriptor_scripts(&caps["heir"])?;
                let min_locktime: u16 = caps["rlock"].parse().map_err(|e| {
                    log::info!("Failed to parse min_locktime: {e}");
                    Error::InvalidScriptFragments("v1")
                })?;
                let absolute_locktime: u64 = caps["alock"].parse().map_err(|e| {
                    log::info!("Failed to parse absolute_locktime: {e}");
                    Error::InvalidScriptFragments("v1")
                })?;
                Ok((heir_config, min_locktime, absolute_locktime))
            })
            .collect::<crate::errors::Result<Vec<_>>>()?;
        // Sort that by the absolute lock time
        heritage_parts.sort_by_key(|e| e.2);

        // The order must also be respected for min_locktimes and they are all successive multiples of the first one
        let min_lock_time_blocks = heritage_parts[0].1;

        if !heritage_parts
            .iter()
            .zip(1u16..)
            .all(|((_, rlock, _), mult)| {
                *rlock == min_lock_time_blocks.checked_mul(mult).unwrap_or(u16::MAX)
            })
        {
            log::info!("Failed the min_lock_time serie control");
            return Err(Error::InvalidScriptFragments("v1"));
        }

        if min_lock_time_blocks % BLOCKS_IN_A_DAY != 0 {
            log::info!("Failed the min_lock_time serie control, {min_lock_time_blocks} is not divisible by {BLOCKS_IN_A_DAY}");
            return Err(Error::InvalidScriptFragments("v1"));
        }
        let min_lock_time_days = min_lock_time_blocks / BLOCKS_IN_A_DAY;

        // The minimum absolute locktime
        let min_absolute_lock_timestamp = heritage_parts[0].2;
        // Assume 1yr lock for the first heir to get the reference timestamp
        let reference_time = min_absolute_lock_timestamp - SEC_IN_A_DAY * 365;

        // Now compute every Heritage by taking the distance
        // between reference_time and absolute_locktime of
        // each heritage_parts
        let heritages = heritage_parts
            .into_iter()
            .map(|(heir_config, _, absolute_locktime)| {
                let time_diff_in_secs = absolute_locktime - reference_time;
                if time_diff_in_secs % SEC_IN_A_DAY != 0 {
                    log::info!("Failed heritages creation, {time_diff_in_secs} sec is not an exact amount of days");
                    return Err(Error::InvalidScriptFragments("v1"));
                }
                Ok(Heritage {
                    heir_config,
                    time_lock: Days((time_diff_in_secs/SEC_IN_A_DAY) as u16),
                })
            })
            .collect::<crate::errors::Result<Vec<_>>>()?;

        Ok(HeritageConfigBuilder::default()
            .reference_time(reference_time)
            .minimum_lock_time(min_lock_time_days)
            .expand_heritages(heritages)
            .build_v1())
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
    /// Adds a single heritage configuration to the builder.
    ///
    /// This method appends a new heritage entry to the list of heritages that will be
    /// included in the final configuration.
    pub fn add_heritage(mut self, heritage: Heritage) -> Self {
        self.heritages.push(heritage);
        self
    }

    /// Adds multiple heritage configurations from an iterator.
    ///
    /// This method is useful when you have a collection of heritage configurations
    /// that need to be added to the builder at once.
    pub fn expand_heritages(mut self, heritages: impl IntoIterator<Item = Heritage>) -> Self {
        self.heritages
            .append(&mut Vec::from_iter(heritages.into_iter()));
        self
    }

    /// Sets the reference timestamp for the heritage configuration.
    ///
    /// The reference timestamp is used as the basis for calculating absolute lock times
    /// for heritage transactions. It must be greater than the Bitcoin `LOCK_TIME_THRESHOLD`
    /// (500,000,000) to ensure it's interpreted as a Unix timestamp rather than a block height.
    ///
    /// # Panics
    ///
    /// Panics if `reference_time` is less than or equal to `LOCK_TIME_THRESHOLD` (500,000,000),
    /// as this would change the semantics of absolute lock time from timestamp to block height.
    pub fn reference_time(mut self, reference_time: u64) -> Self {
        self.reference_timestamp = ReferenceTimestamp::new(reference_time);
        self
    }

    /// Sets the minimum lock time in days for heritage transactions.
    ///
    /// The minimum lock time acts as a safety measure to prevent accidental immediate
    /// inheritance claims. The required minimum varies by network:
    /// - Bitcoin mainnet: minimum 10 days
    /// - Test networks: minimum 1 day
    ///
    /// # Panics
    ///
    /// Panics if:
    /// - On Bitcoin mainnet: `minimum_lock_time` is less than 10 days
    /// - On other networks: `minimum_lock_time` is less than 1 day
    pub fn minimum_lock_time(mut self, minimum_lock_time: u16) -> Self {
        self.minimum_lock_time = MinimumLockTime::new(Days(minimum_lock_time));
        self
    }

    /// Builds the final heritage configuration as a version-agnostic type.
    ///
    /// This method wraps the V1-specific configuration in the generic `HeritageConfig`
    /// enum, allowing for future version compatibility.
    ///
    /// # Examples
    ///
    /// ```
    /// use btc_heritage::heritage_config::v1::HeritageConfigBuilder;
    ///
    /// let config = HeritageConfigBuilder::default()
    ///     .reference_time(1702900800)
    ///     .minimum_lock_time(30)
    ///     .build();
    /// ```
    pub fn build(self) -> super::HeritageConfig {
        super::HeritageConfig(super::InnerHeritageConfig::V1(self.build_v1()))
    }

    /// Builds the V1-specific heritage configuration.
    ///
    /// This method creates the final `HeritageConfig` struct, normalizing the heritage
    /// list to ensure proper ordering and validation. The normalization process sorts
    /// heritages by lock time and validates that there are no duplicate configurations.
    pub fn build_v1(self) -> HeritageConfig {
        // Create Heritages from the Vec of Heritage and normalize it
        let mut heritages = Heritages(self.heritages);
        heritages.normalize();
        HeritageConfig {
            heritages,
            reference_timestamp: self.reference_timestamp,
            minimum_lock_time: self.minimum_lock_time,
        }
    }
}

#[derive(Debug)]
pub(crate) struct HeritageExplorer<'a> {
    heritage_config: &'a HeritageConfig,
    heritage_index: usize,
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

    fn has_fingerprint(&self, fingerprint: Fingerprint) -> bool {
        self.heritage_config.heritages.0[self.heritage_index]
            .get_heir_config()
            .fingerprint()
            == fingerprint
    }

    fn get_miniscript_expression<'b>(
        &self,
        origins: impl Iterator<Item = (&'b Fingerprint, &'b DerivationPath)>,
    ) -> String {
        self.heritage_config
            .get_concrete_heritage_script_string(self.heritage_index, origins)
    }
}

#[cfg(test)]
#[allow(irrefutable_let_patterns)]
mod tests {

    use crate::heritage_config::FromDescriptorScripts;
    use crate::tests::*;

    use super::{
        super::{HeritageConfig as VHeritageConfig, InnerHeritageConfig as IHC},
        HeritageConfig as HeritageConfigV1, HeritageConfigBuilder, LOCK_TIME_THRESHOLD,
    };

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
            hc1.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap(),
            hc2.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap()
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
            hc1.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap(),
            hc2.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap()
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
            hc1.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap(),
            hc2.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap()
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
            hc1.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap(),
            hc2.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap()
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
            hc.descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap()
        );

        assert_eq!(
            get_test_heritage_config(TestHeritageConfig::BackupWifeBro)
                .descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap(),
            format!(
                "{{and_v(v:pk({backup_pubkey}),and_v(v:older(12960),after(1794608000))),\
                {{and_v(v:pk({wife_pubkey}),and_v(v:older(25920),after(1797632000))),\
                and_v(v:pk({brother_pubkey}),and_v(v:older(38880),after(1800656000)))}}}}"
            )
        );
    }

    #[test]
    fn fragment_scripts() {
        // Test empty fragment
        assert_eq!(super::fragment_scripts(""), Vec::<&str>::new());

        // Single fragment
        assert_eq!(
            super::fragment_scripts("and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1794608000)))"),
            vec!["and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1794608000)))"]
        );

        // Made-up invalid miniscripts to test behavior
        let t = "{()(,),(,)}";
        let e = vec!["()(,)", "(,)"];
        assert_eq!(super::fragment_scripts(t), e, "{t}");

        let t = "{{(a(a,a)),(b)(b,b)},(,)}";
        let e = vec!["(a(a,a))", "(b)(b,b)", "(,)"];
        assert_eq!(super::fragment_scripts(t), e, "{t}");

        let t = "{(,),{((,)),()(,)}}";
        let e = vec!["(,)", "((,))", "()(,)"];
        assert_eq!(super::fragment_scripts(t), e, "{t}");

        let t = "{{a(b(c,d)),e(f(g,h))},{i(j(k,l)),m(n(o,p))}}";
        let e = vec!["a(b(c,d))", "e(f(g,h))", "i(j(k,l))", "m(n(o,p))"];
        assert_eq!(super::fragment_scripts(t), e, "{t}");

        let t = "{{a(b,c),{a(b,c),a(b,c)}},{{a(b,c),a(b,c)},{a(b,c),{a(b,c),a(b,c)}}}}";
        let e = vec![
            "a(b,c)", "a(b,c)", "a(b,c)", "a(b,c)", "a(b,c)", "a(b,c)", "a(b,c)", "a(b,c)",
        ];
        assert_eq!(super::fragment_scripts(t), e, "{t}");

        // Test a real case
        let script = "{and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1794608000))),{and_v(v:pk([c907dcb9/86'/1'/1751476594'/0/0]029d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf),and_v(v:older(25920),after(1797632000))),and_v(v:pk([767e581a/86'/1'/1751476594'/0/0]03f49679ef0089dda208faa970d7491cca8334bbe2ca541f527a6d7adf06a53e9e),and_v(v:older(38880),after(1800656000)))}}";
        let fragments = super::fragment_scripts(script);
        assert_eq!(fragments, vec![
        "and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1794608000)))",
        "and_v(v:pk([c907dcb9/86'/1'/1751476594'/0/0]029d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf),and_v(v:older(25920),after(1797632000)))",
        "and_v(v:pk([767e581a/86'/1'/1751476594'/0/0]03f49679ef0089dda208faa970d7491cca8334bbe2ca541f527a6d7adf06a53e9e),and_v(v:older(38880),after(1800656000)))"
        ])
    }

    #[test]
    fn from_descriptor_scripts() {
        let hc1 = HeritageConfigV1::builder()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(90)
            .build_v1();

        let hc2 = HeritageConfigV1::builder()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072001)
            .minimum_lock_time(90)
            .build_v1();
        // different minimum_lock_time
        let hc3 = HeritageConfigV1::builder()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(get_test_heritage(TestHeritage::Wife))
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(91)
            .build_v1();
        // Different heritage timelock
        let hc4 = HeritageConfigV1::builder()
            .add_heritage(get_test_heritage(TestHeritage::Backup))
            .add_heritage(
                super::Heritage::new(get_test_heritage(TestHeritage::Wife).heir_config.clone())
                    .time_lock(900),
            )
            .add_heritage(get_test_heritage(TestHeritage::Brother))
            .reference_time(1763072000)
            .minimum_lock_time(91)
            .build_v1();

        // We very that it works and it is stable
        // Meaning that the Heritage config recovered from a script fragment should produce the exact same fragment
        for hc in [hc1, hc2, hc3, hc4] {
            let fragment = hc
                .descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap();
            let restored_hc = HeritageConfigV1::from_descriptor_scripts(&fragment).unwrap();
            let restored_fragment = restored_hc
                .descriptor_taptree_miniscript_expression_for_child(None)
                .unwrap();
            assert_eq!(fragment, restored_fragment, "Failed for {fragment}");
        }
    }

    #[test]
    #[should_panic(expected = "reference_time cannot be less or equal to")]
    fn reference_time_panics_with_threshold_value() {
        HeritageConfigBuilder::default().reference_time(LOCK_TIME_THRESHOLD as u64);
    }

    #[test]
    #[should_panic(expected = "reference_time cannot be less or equal to")]
    fn reference_time_panics_with_low_value() {
        HeritageConfigBuilder::default().reference_time(400_000_000);
    }

    #[test]
    fn reference_time_accepts_valid_timestamp() {
        // Should not panic with a valid timestamp (greater than threshold)
        let builder = HeritageConfigBuilder::default().reference_time(1702900800);
        assert_eq!(builder.reference_timestamp.as_u64(), 1702900800);
    }

    #[test]
    #[should_panic(expected = "minimum_lock_time cannot be less than 1 day")]
    fn minimum_lock_time_panics_with_zero() {
        // This test assumes we're not on Bitcoin mainnet
        // If we are on mainnet, the panic message will be different but it will still panic
        HeritageConfigBuilder::default().minimum_lock_time(0);
    }

    #[test]
    #[should_panic(expected = "reference_time cannot be less or equal to")]
    fn reference_time_deser_panics_with_threshold_value() {
        let h1 = get_test_heritage(TestHeritage::Backup).time_lock(90);

        let VHeritageConfig(IHC::V1(_)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                "version": "v1",
                "heritages":[
                    {}
                ],
                "minimum_lock_time":90,"reference_timestamp":{LOCK_TIME_THRESHOLD}
            }}"#,
            serde_json::to_string(&h1).unwrap(),
        ))
        .unwrap();
    }

    #[test]
    #[should_panic(expected = "reference_time cannot be less or equal to")]
    fn reference_time_deser_panics_with_low_value() {
        let h1 = get_test_heritage(TestHeritage::Backup).time_lock(90);

        let VHeritageConfig(IHC::V1(_)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                    "version": "v1",
                    "heritages":[
                        {}
                    ],
                    "minimum_lock_time":90,"reference_timestamp":400000000
                }}"#,
            serde_json::to_string(&h1).unwrap(),
        ))
        .unwrap();
    }

    #[test]
    #[should_panic(expected = "minimum_lock_time cannot be less than 1 day")]
    fn minimum_lock_deser_time_panics_with_zero() {
        // This test assumes we're not on Bitcoin mainnet
        // If we are on mainnet, the panic message will be different but it will still panic
        let h1 = get_test_heritage(TestHeritage::Backup).time_lock(90);

        let VHeritageConfig(IHC::V1(_)): VHeritageConfig = serde_json::from_str(&format!(
            r#"{{
                    "version": "v1",
                    "heritages":[
                        {}
                    ],
                    "minimum_lock_time":0,"reference_timestamp":1702900800
                }}"#,
            serde_json::to_string(&h1).unwrap(),
        ))
        .unwrap();
    }
}
