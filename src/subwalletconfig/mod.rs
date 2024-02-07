use crate::{accountxpub::AccountXPub, heritageconfig::HeritageConfig, utils};

use bdk::{
    bitcoin::{
        bip32::{ChildNumber, DerivationPath},
        secp256k1::Secp256k1,
    },
    database::BatchDatabase,
    keys::{DerivableKey, DescriptorKey},
    miniscript::{descriptor::DescriptorXKey, DescriptorPublicKey, Tap},
    Wallet,
};

pub use bdk::{bitcoin::psbt::PartiallySignedTransaction, FeeRate};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
struct SubwalletFirstUseTime(u64);
impl Default for SubwalletFirstUseTime {
    fn default() -> Self {
        // The current timestamp
        // In effet, this is "Today at 12:00 (24H)"
        Self(utils::timestamp_now())
    }
}

pub type SubwalletId = u32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubwalletConfig {
    subwallet_id: SubwalletId,
    ext_descriptor: String,
    change_descriptor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subwallet_firstuse_time: Option<SubwalletFirstUseTime>,
    account_xpub: AccountXPub,
    heritage_config: HeritageConfig,
}

impl SubwalletConfig {
    pub fn new(
        subwallet_id: SubwalletId,
        account_xpub: AccountXPub,
        heritage_config: HeritageConfig,
    ) -> Self {
        let secp = Secp256k1::new();

        log::debug!("Wallet::new - wallet_id={subwallet_id}");

        let descriptor = account_xpub.descriptor_public_key();
        log::debug!("account descriptor={descriptor}");
        let (fingerprint, derivation_path, account_xpub_key) = match descriptor {
            DescriptorPublicKey::XPub(DescriptorXKey {
                origin: Some((fingerprint, path)),
                xkey,
                ..
            }) => (fingerprint, path, xkey),
            _ => panic!("Invalid key variant"),
        };
        log::debug!("Wallet::new - fingerprint={fingerprint}");
        log::debug!("Wallet::new - derivation_path={derivation_path}");
        log::debug!("Wallet::new - account_xpub_key={account_xpub_key}");

        let mut descriptor_iterator = derivation_path.normal_children().take(2).map(|child_path| {
            let deriv_path = DerivationPath::from(
                &child_path
                    .into_iter()
                    .cloned()
                    .collect::<Vec<ChildNumber>>()[derivation_path.len()..],
            );
            let xpub = account_xpub_key.derive_pub(&secp, &deriv_path).unwrap();
            let origin = Some((*fingerprint, child_path));
            let descriptor: DescriptorKey<Tap> = xpub
                .into_descriptor_key(origin, DerivationPath::default())
                .unwrap();
            if let DescriptorKey::Public(desc_pubkey, ..) = descriptor {
                match heritage_config.descriptor_taptree_miniscript_expression() {
                    Some(script_paths) => format!("tr({desc_pubkey},{script_paths})"),
                    None => format!("tr({desc_pubkey})"),
                }
            } else {
                panic!("Invalid key variant")
            }
        });

        let (ext_descriptor, change_descriptor) = (
            descriptor_iterator.next().unwrap(),
            descriptor_iterator.next().unwrap(),
        );
        log::debug!("Wallet::new - ext_descriptor={ext_descriptor}");
        log::debug!("Wallet::new - change_descriptor={change_descriptor}");

        Self {
            subwallet_id,
            ext_descriptor,
            change_descriptor,
            subwallet_firstuse_time: None,
            account_xpub,
            heritage_config,
        }
    }

    pub fn get_subwallet<DB: BatchDatabase>(&self, subdatabase: DB) -> Wallet<DB> {
        Wallet::new(
            self.ext_descriptor.as_str(),
            Some(self.change_descriptor.as_str()),
            *utils::bitcoin_network_from_env(),
            subdatabase,
        )
        .expect("failed because descriptors checksums are inconsistent with previous DB values")
    }

    pub fn subwallet_id(&self) -> SubwalletId {
        self.subwallet_id
    }

    pub fn subwallet_firstuse_time(&self) -> Option<u64> {
        self.subwallet_firstuse_time.map(|e| e.0)
    }

    pub fn mark_subwallet_firstuse(&mut self) -> Result<()> {
        if self.subwallet_firstuse_time.is_some() {
            bail!("Subwallet has already been used")
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

    pub fn ext_descriptor(&self) -> &str {
        &self.ext_descriptor
    }

    pub fn change_descriptor(&self) -> &str {
        &self.change_descriptor
    }

    /// Consume the [SubwalletConfig] in order to retrieve its [SubwalletId],
    /// [AccountXPub] and [HeritageConfig] without having to clone anything
    pub fn into_parts(self) -> (SubwalletId, AccountXPub, HeritageConfig) {
        (self.subwallet_id, self.account_xpub, self.heritage_config)
    }

    #[cfg(test)]
    fn get_test_subwallet(&self) -> Wallet<bdk::database::AnyDatabase> {
        bdk::wallet::get_funded_wallet(self.ext_descriptor.as_str()).0
    }
    #[cfg(test)]
    pub(crate) fn set_subwallet_firstuse(&mut self, ts: u64) {
        self.subwallet_firstuse_time = Some(SubwalletFirstUseTime(ts));
    }
}

#[cfg(test)]
mod tests {

    use std::{collections::BTreeMap, str::FromStr};

    use bdk::{
        bitcoin::{
            bip32::Fingerprint,
            secp256k1::XOnlyPublicKey,
            taproot::{TapLeafHash, TapNodeHash},
        },
        database::AnyDatabase,
        wallet::AddressIndex,
        Balance, KeychainKind,
    };

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
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 0)
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
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 0)
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY2, 1)
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
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY1, 0)
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address(TestHeritageConfig::BackupWifeY1, 1)
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
            get_default_test_subwallet_config_expected_address(
                TestHeritageConfig::BackupWifeBro,
                0
            )
        );
        assert_eq!(
            wallet
                .get_address(AddressIndex::Peek(1))
                .unwrap()
                .to_string(),
            get_default_test_subwallet_config_expected_address(
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
}
