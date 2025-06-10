use core::str::FromStr;

use crate::{
    account_xpub::AccountXPub,
    errors::{Error, Result},
    heritage_config::{FromDescriptorScripts, HeritageConfig},
    miniscript::{Descriptor, DescriptorPublicKey},
    utils, SubwalletDescriptorBackup,
};

use bdk::{database::BatchDatabase, Wallet};
use serde::{Deserialize, Serialize};

pub use crate::bitcoin::psbt::PartiallySignedTransaction;

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
struct SubwalletFirstUseTime(u64);
impl Default for SubwalletFirstUseTime {
    fn default() -> Self {
        // The current timestamp
        Self(utils::timestamp_now())
    }
}

pub type SubwalletId = u32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubwalletConfig {
    ext_descriptor: Descriptor<DescriptorPublicKey>,
    change_descriptor: Descriptor<DescriptorPublicKey>,
    account_xpub: AccountXPub,
    heritage_config: HeritageConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subwallet_firstuse_time: Option<SubwalletFirstUseTime>,
}

impl SubwalletConfig {
    // WARNING: We MUST never change that
    // If we DO change it, old backups will no longer map to the right addresses
    // At the very least, EXTENSIVE communication should be done so users
    // re-create backups or modify the existing ones to specify explicitely
    // the ext_index and change_index values
    pub const DEFAULT_EXTERNAL_INDEX: u32 = 0;
    pub const DEFAULT_CHANGE_INDEX: u32 = 1;

    pub fn new(account_xpub: AccountXPub, heritage_config: HeritageConfig) -> Self {
        log::debug!(
            "SubwalletConfig::new - \
        account_xpub={account_xpub} heritage_config={heritage_config:?}"
        );
        Self::new_with_custom_indexes(
            account_xpub,
            heritage_config,
            Self::DEFAULT_EXTERNAL_INDEX,
            Self::DEFAULT_CHANGE_INDEX,
        )
    }

    pub fn new_with_custom_indexes(
        account_xpub: AccountXPub,
        heritage_config: HeritageConfig,
        external_index: u32,
        change_index: u32,
    ) -> Self {
        log::debug!(
            "SubwalletConfig::new_with_custom_indexes - \
        account_xpub={account_xpub} heritage_config={heritage_config:?} \
        external_index={external_index} change_index={change_index}"
        );

        let (ext_descriptor, change_descriptor) = Self::create_descriptors(
            &account_xpub,
            &heritage_config,
            external_index,
            change_index,
        );
        log::debug!("SubwalletConfig::new_with_custom_indexes - ext_descriptor={ext_descriptor}");
        log::debug!(
            "SubwalletConfig::new_with_custom_indexes - change_descriptor={change_descriptor}"
        );

        Self {
            ext_descriptor,
            change_descriptor,
            subwallet_firstuse_time: None,
            account_xpub,
            heritage_config,
        }
    }

    pub fn create_descriptors(
        account_xpub: &AccountXPub,
        heritage_config: &HeritageConfig,
        external_index: u32,
        change_index: u32,
    ) -> (
        Descriptor<DescriptorPublicKey>,
        Descriptor<DescriptorPublicKey>,
    ) {
        let mut descriptor_iterator = [external_index, change_index].into_iter().map(|index| {
            let descriptor_public_key = account_xpub.child_descriptor_public_key(index);
            let descriptor_taptree_miniscript_expression =
                heritage_config.descriptor_taptree_miniscript_expression_for_child(Some(index));
            let descriptor_string = match &descriptor_taptree_miniscript_expression {
                Some(script_paths) => format!("tr({descriptor_public_key},{script_paths})"),
                None => format!("tr({descriptor_public_key})"),
            };
            Descriptor::<DescriptorPublicKey>::from_str(&descriptor_string)
                .expect("we produce valid descriptor strings")
        });
        (
            descriptor_iterator.next().unwrap(),
            descriptor_iterator.next().unwrap(),
        )
    }

    pub fn get_subwallet<DB: BatchDatabase>(&self, subdatabase: DB) -> Wallet<DB> {
        Wallet::new(
            self.ext_descriptor.clone(),
            Some(self.change_descriptor.clone()),
            *utils::bitcoin_network_from_env(),
            subdatabase,
        )
        .expect("failed because descriptors checksums are inconsistent with previous DB values")
    }

    pub fn subwallet_id(&self) -> SubwalletId {
        self.account_xpub.descriptor_id()
    }

    pub fn subwallet_firstuse_time(&self) -> Option<u64> {
        self.subwallet_firstuse_time.map(|e| e.0)
    }

    pub fn mark_subwallet_firstuse(&mut self) -> Result<()> {
        if self.subwallet_firstuse_time.is_some() {
            log::error!("Subwallet has already been used");
            return Err(Error::SubwalletConfigAlreadyMarkedUsed);
        }
        self.subwallet_firstuse_time = Some(SubwalletFirstUseTime::default());
        Ok(())
    }

    pub fn account_xpub(&self) -> &AccountXPub {
        &self.account_xpub
    }

    pub fn heritage_config(&self) -> &HeritageConfig {
        &self.heritage_config
    }

    pub fn ext_descriptor(&self) -> &Descriptor<DescriptorPublicKey> {
        &self.ext_descriptor
    }

    pub fn change_descriptor(&self) -> &Descriptor<DescriptorPublicKey> {
        &self.change_descriptor
    }

    /// Consume the [SubwalletConfig] in order to retrieve its
    /// [AccountXPub] and [HeritageConfig] without having to clone anything
    pub fn into_parts(self) -> (AccountXPub, HeritageConfig) {
        (self.account_xpub, self.heritage_config)
    }

    #[cfg(test)]
    fn get_test_subwallet(&self) -> Wallet<bdk::database::AnyDatabase> {
        bdk::wallet::get_funded_wallet(self.ext_descriptor.to_string().as_str()).0
    }
    #[cfg(test)]
    pub(crate) fn set_subwallet_firstuse(&mut self, ts: u64) {
        self.subwallet_firstuse_time = Some(SubwalletFirstUseTime(ts));
    }
}

/// Match a whole Taproot descriptor and allow to retrieve the key and, if present, the scripts
fn re_descriptor() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^(?<desc>tr\((?<key>.+?)(?:,(?<scripts>.+))?\))(:?#[a-z0-9]{8})?$")
            .unwrap()
    })
}
/// Match an [AccountXPub] string and allow to separate the origin/key and the final derivation
fn re_account_xpub() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?<key>\[[0-9a-f]{8}\/86['h]\/[01]['h]\/[0-9]+['h]\][tx]pub[1-9A-HJ-NP-Za-km-z]{79,108})(?<derivation>\/\*\*|(:?\/[0-9]+)*\/\*)",
        )
        .unwrap()
    })
}

impl TryFrom<&SubwalletDescriptorBackup> for SubwalletConfig {
    type Error = Error;

    fn try_from(sdb: &SubwalletDescriptorBackup) -> std::result::Result<Self, Self::Error> {
        // First, both descriptor backups keys must
        // 1 - be the same
        // 2 - the first must be valid a AccountXPub
        // 3 - others must be valid heir configs
        // Note: Alternate display prevents the checksum to go in
        let ext_desc = format!("{:#}", sdb.external_descriptor);
        let chg_desc = format!("{:#}", sdb.change_descriptor);

        // Replace all the [AccountXPub]s in both descriptors by removing the post-derivation and verify the two resulting string matches
        let ext_desc = re_account_xpub().replace_all(&ext_desc, "${key}/*");
        let chg_desc = re_account_xpub().replace_all(&chg_desc, "${key}/*");
        if ext_desc != chg_desc {
            log::error!("external and change descriptor are not compatible");
            log::error!("external: {ext_desc}");
            log::error!("change: {chg_desc}");
            return Err(Error::InvalidBackup(
                "external and change descriptor are not compatible",
            ));
        }
        // Now we can work only with one of them
        let desc = ext_desc;
        let capts = re_descriptor()
            .captures(&desc)
            .ok_or(Error::InvalidBackup("descriptors are not Tr"))?;

        let account_xpub = AccountXPub::try_from(&capts["key"])?;
        let scripts = capts
            .name("scripts")
            .map(|cap| cap.as_str())
            .unwrap_or_default();
        let heritage_config = HeritageConfig::from_descriptor_scripts(scripts)?;

        Ok(Self {
            ext_descriptor: sdb.external_descriptor.clone(),
            change_descriptor: sdb.change_descriptor.clone(),
            account_xpub,
            heritage_config,
            subwallet_firstuse_time: sdb.first_use_ts.map(|ts| SubwalletFirstUseTime(ts)),
        })
    }
}

#[cfg(test)]
mod tests {

    use crate::bitcoin::{
        bip32::{DerivationPath, Fingerprint},
        secp256k1::XOnlyPublicKey,
        taproot::{TapLeafHash, TapNodeHash},
    };
    use bdk::{database::AnyDatabase, wallet::AddressIndex, Balance, KeychainKind};
    use core::str::FromStr;
    use std::collections::BTreeMap;

    use crate::{tests::*, utils::string_to_address};

    use super::*;

    fn setup_test_wallet() -> Wallet<AnyDatabase> {
        let wallet_config = get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY2);
        let wallet = wallet_config.get_test_subwallet();

        let addr1 = wallet
            .get_address(AddressIndex::Peek(0))
            .unwrap()
            .to_string();
        assert_eq!(
            addr1,
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeY2,
                0
            )
        );

        wallet
    }

    #[test]
    fn wallet_expected_values() {
        let wallet_config = get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY2);
        let wallet = wallet_config.get_test_subwallet();
        assert_eq!(
            wallet
                .get_descriptor_for_keychain(KeychainKind::External)
                .to_string(),
            get_default_test_subwallet_config_expected_external_descriptor(
                TestHeritageConfig::BackupWifeY2
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(0))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeY2,
                0
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeY2,
                1
            )
        );

        let wallet_config = get_default_test_subwallet_config(TestHeritageConfig::BackupWifeY1);
        let wallet = wallet_config.get_test_subwallet();
        assert_eq!(
            wallet
                .get_descriptor_for_keychain(KeychainKind::External)
                .to_string(),
            get_default_test_subwallet_config_expected_external_descriptor(
                TestHeritageConfig::BackupWifeY1
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(0))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeY1,
                0
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeY1,
                1
            )
        );

        let wallet_config = get_default_test_subwallet_config(TestHeritageConfig::BackupWifeBro);
        let wallet = wallet_config.get_test_subwallet();
        assert_eq!(
            wallet
                .get_descriptor_for_keychain(KeychainKind::External)
                .to_string(),
            get_default_test_subwallet_config_expected_external_descriptor(
                TestHeritageConfig::BackupWifeBro
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(0))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeBro,
                0
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address_without_origin(
                TestHeritageConfig::BackupWifeBro,
                1
            )
        );
    }

    #[test]
    fn balance() {
        let wallet = setup_test_wallet();
        let expected_balance = Balance {
            immature: 0,
            trusted_pending: 0,
            untrusted_pending: 0,
            confirmed: 50_000,
        };
        assert_eq!(wallet.get_balance().unwrap(), expected_balance);
    }

    #[test]
    fn psbt_generation() {
        let wallet = setup_test_wallet();

        let mut tx_builder = wallet.build_tx();
        tx_builder
            .set_recipients(vec![
                (
                    string_to_address(PKH_EXTERNAL_RECIPIENT_ADDR)
                        .unwrap()
                        .script_pubkey(),
                    1000,
                ),
                (
                    string_to_address(WPKH_EXTERNAL_RECIPIENT_ADDR)
                        .unwrap()
                        .script_pubkey(),
                    2000,
                ),
                (
                    string_to_address(TR_EXTERNAL_RECIPIENT_ADDR)
                        .unwrap()
                        .script_pubkey(),
                    3000,
                ),
            ])
            .policy_path(
                BTreeMap::from([(
                    wallet.policies(KeychainKind::External).unwrap().unwrap().id,
                    vec![0],
                )]),
                KeychainKind::External,
            );
        let psbt = tx_builder.finish().unwrap().0;

        // This PSBT has 1 input
        assert_eq!(psbt.inputs.len(), 1);
        // Tap Internal key is expected
        assert!(psbt.inputs[0].tap_internal_key.is_some_and(|ik| ik
            == XOnlyPublicKey::from_str(
                "ea7877acac8ca3128e09e77236c840e1a3fc23297f8e45ebee53973f311cf177"
            )
            .unwrap()));
        // Tap Merkle root is expected
        assert!(psbt.inputs[0].tap_merkle_root.is_some_and(|tnh| tnh
            == TapNodeHash::from_str(
                "2ab2e3fcb5ae9acbf80ea8c4cbe24f0f5ee132411e596b9ed1ffa5d8640c7424"
            )
            .unwrap()));
        // Key paths are expected
        assert!(psbt.inputs[0]
            .tap_key_origins
            .get(&psbt.inputs[0].tap_internal_key.unwrap())
            .is_some_and(
                |(tap_leaf_hash, (key_fingerprint, derivation_path))| tap_leaf_hash.is_empty()
                    && *key_fingerprint == Fingerprint::from_str("9c7088e3").unwrap()
                    && *derivation_path == DerivationPath::from_str("m/86'/1'/0'/0/0").unwrap()
            ));
        assert!(psbt.inputs[0]
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    "9d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf"
                )
                .unwrap()
            )
            .is_some_and(
                |(tap_leaf_hash, (key_fingerprint, derivation_path))| tap_leaf_hash.len() == 1
                    && tap_leaf_hash[0]
                        == TapLeafHash::from_str(
                            "9a9223085f008d333f83061cd1212f4b39558891ccce0be3028ee90345e435f9"
                        )
                        .unwrap()
                    && *key_fingerprint == Fingerprint::from_str("c907dcb9").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str("m/86'/1'/1751476594'/0/0").unwrap()
            ));
        assert!(psbt.inputs[0]
            .tap_key_origins
            .get(
                &XOnlyPublicKey::from_str(
                    "5dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20"
                )
                .unwrap()
            )
            .is_some_and(
                |(tap_leaf_hash, (key_fingerprint, derivation_path))| tap_leaf_hash.len() == 1
                    && tap_leaf_hash[0]
                        == TapLeafHash::from_str(
                            "f34c08381808aa33015b43c6260c5c1368b8aadedd35086ce1677a646d33ab75"
                        )
                        .unwrap()
                    && *key_fingerprint == Fingerprint::from_str("f0d79bf6").unwrap()
                    && *derivation_path
                        == DerivationPath::from_str("m/86'/1'/1751476594'/0/0").unwrap()
            ));

        // 4 outputs
        assert_eq!(psbt.outputs.len(), 4);
        // Output with 1_000 sat is P2PKH
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value == 1_000)
            .all(|e| e.script_pubkey.is_p2pkh()));
        // Output with 2_000 sat is P2WPKH
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value == 2_000)
            .all(|e| e.script_pubkey.is_v0_p2wpkh()));
        // Outputs with 3_000 sat and more are P2TR
        assert!(psbt
            .unsigned_tx
            .output
            .iter()
            .filter(|e| e.value >= 3_000)
            .all(|e| e.script_pubkey.is_v1_p2tr()));
    }

    #[test]
    fn from_subwallet_descriptor_backup() {
        // Invalid because descriptorschecksum
        assert!(Descriptor::<DescriptorPublicKey>::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))#0u0qafga").is_err());

        // Invalid because descriptors are not Tr
        let invalid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("wpkh([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*)").unwrap(),
            change_descriptor: Descriptor::from_str("wpkh([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/1/*)").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        assert!(SubwalletConfig::try_from(&invalid_backup).is_err());

        // Invalid because descriptors are not compatible (not the same Account)
        let invalid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/1']tpubDDpFTt9TRJho32GzE4j9D5KHTQtww39w1AJkF9pFW435Zg13dFfzHmDD2iEDRkXhZJm1rxZFy1c4PhcWNLvW2ouEM51SULxXvVAkwhFaeuS/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(8640),after(1783072800))))").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        assert!(SubwalletConfig::try_from(&invalid_backup).is_err());

        // Invalid because descriptors are not compatible (not the same scripts)
        let invalid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(8641),after(1783072800))))").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        assert!(SubwalletConfig::try_from(&invalid_backup).is_err());

        // Invalid because descriptors are not compatible (not the same scripts)
        let invalid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(8640),after(1783072801))))").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        assert!(SubwalletConfig::try_from(&invalid_backup).is_err());

        // Invalid because descriptors are not compatible (not the same scripts)
        let invalid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/1/*,and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(8640),after(1783072800))))").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        assert!(SubwalletConfig::try_from(&invalid_backup).is_err());

        // Valid
        let valid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(8640),after(1783072800))))#78zjz03g").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/0']tpubDDpFTt9TRJhnzh4NfWHN87p8skizWRpq86h6tc5rp9pK1DTLhicYiEumTfDF56DxcrQi6dnq8pCpcwS7RvTZ8vXjTa5LQSXDSKoghvcqhpa/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(8640),after(1783072800))))#0u0qafga").unwrap(),
            first_use_ts: Some(1720879341),
            last_external_index: None,
            last_change_index: None,
        };
        let swc = SubwalletConfig::try_from(&valid_backup);
        assert!(swc.is_ok(), "{}", swc.err().unwrap());
        let swc = swc.unwrap();
        assert_eq!(swc.subwallet_id(), 0);
        assert_eq!(swc.subwallet_firstuse_time(), Some(1720879341));
        assert_eq!(swc.account_xpub().descriptor_id(), 0);
        assert_eq!(swc.heritage_config().iter_heir_configs().count(), 1);

        // Valid
        let valid_backup = SubwalletDescriptorBackup {
            external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/1']tpubDDpFTt9TRJho32GzE4j9D5KHTQtww39w1AJkF9pFW435Zg13dFfzHmDD2iEDRkXhZJm1rxZFy1c4PhcWNLvW2ouEM51SULxXvVAkwhFaeuS/0/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1737706192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1753258192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1768810192)))}})#hjqtx6s0").unwrap(),
            change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/1']tpubDDpFTt9TRJho32GzE4j9D5KHTQtww39w1AJkF9pFW435Zg13dFfzHmDD2iEDRkXhZJm1rxZFy1c4PhcWNLvW2ouEM51SULxXvVAkwhFaeuS/1/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1737706192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1753258192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1768810192)))}})#vryrfyh7").unwrap(),
            first_use_ts: Some(1706600000),
            last_external_index: None,
            last_change_index: None,
        };
        let swc = SubwalletConfig::try_from(&valid_backup);
        assert!(swc.is_ok(), "{}", swc.err().unwrap());
        let swc = swc.unwrap();
        assert_eq!(swc.subwallet_id(), 1);
        assert_eq!(swc.subwallet_firstuse_time(), Some(1706600000));
        assert_eq!(swc.account_xpub().descriptor_id(), 1);
        assert_eq!(swc.heritage_config().iter_heir_configs().count(), 3);

        // Valid
        let valid_backup = SubwalletDescriptorBackup {
                    external_descriptor: Descriptor::from_str("tr([44990794/86'/1'/1']tpubDDpFTt9TRJho32GzE4j9D5KHTQtww39w1AJkF9pFW435Zg13dFfzHmDD2iEDRkXhZJm1rxZFy1c4PhcWNLvW2ouEM51SULxXvVAkwhFaeuS/0/*)").unwrap(),
                    change_descriptor: Descriptor::from_str("tr([44990794/86'/1'/1']tpubDDpFTt9TRJho32GzE4j9D5KHTQtww39w1AJkF9pFW435Zg13dFfzHmDD2iEDRkXhZJm1rxZFy1c4PhcWNLvW2ouEM51SULxXvVAkwhFaeuS/1/*)").unwrap(),
                    first_use_ts: Some(1706600000),
                    last_external_index: None,
                    last_change_index: None,
                };
        let swc = SubwalletConfig::try_from(&valid_backup);
        assert!(swc.is_ok(), "{}", swc.err().unwrap());
        let swc = swc.unwrap();
        assert_eq!(swc.subwallet_id(), 1);
        assert_eq!(swc.subwallet_firstuse_time(), Some(1706600000));
        assert_eq!(swc.account_xpub().descriptor_id(), 1);
        assert_eq!(swc.heritage_config().iter_heir_configs().count(), 0);
    }
}
