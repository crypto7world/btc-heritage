use core::str::FromStr;

use bitcoin::hex::{Case, DisplayHex, FromHex};
use btc_heritage::{AccountXPub, AccountXPubId, SubwalletDescriptorBackup};
use ledger_bitcoin_client::{WalletPolicy, WalletPubKey};
use serde::{Deserialize, Serialize};

use crate::errors::Error;

fn re_descriptor() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^(?<desc>tr\((?<key>.+?)(?:,(?<scripts>.+))?\))(:?#[a-z0-9]{8})?$")
            .unwrap()
    })
}

fn re_pk() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(:?pkh?|pk_[kh])\((?<key>.+?)\)").unwrap())
}

fn re_account_xpub() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?<key>\[[0-9a-f]{8}\/86['h]\/[01]['h]\/[0-9]+['h]\][tx]pub[1-9A-HJ-NP-Za-km-z]{79,108})(?<derivation>\/\*\*|(:?\/[0-9]+)*\/\*)",
        )
        .unwrap()
    })
}

macro_rules! new_byte_type {
    ($struct_name:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(into = "String", try_from = "String")]
        pub struct $struct_name([u8; 32]);
        impl core::fmt::Display for $struct_name {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(&self.0.to_hex_string(Case::Lower))
            }
        }
        impl From<$struct_name> for String {
            fn from(value: $struct_name) -> Self {
                value.to_string()
            }
        }
        impl From<[u8; 32]> for $struct_name {
            fn from(value: [u8; 32]) -> Self {
                Self(value)
            }
        }
        impl From<$struct_name> for [u8; 32] {
            fn from(value: $struct_name) -> Self {
                value.0
            }
        }
        impl<'a> From<&'a $struct_name> for &'a [u8; 32] {
            fn from(value: &'a $struct_name) -> Self {
                &value.0
            }
        }
        impl TryFrom<&str> for $struct_name {
            type Error = Error;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                let bytes = <[u8; 32]>::from_hex(value).map_err(Error::generic)?;
                Ok(Self(bytes))
            }
        }
        impl TryFrom<String> for $struct_name {
            type Error = Error;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                $struct_name::try_from(value.as_str())
            }
        }
    };
}

new_byte_type!(LedgerPolicyId);
new_byte_type!(LedgerPolicyHMAC);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct LedgerPolicy(String);
impl LedgerPolicy {
    /// Returns the [AccountXPubId] that this policy is for
    pub fn get_account_id(&self) -> AccountXPubId {
        let key = re_account_xpub()
            .find(&self.0)
            .expect("LedgerPolicy ensure the descriptor contains an account_xpub")
            .as_str();
        // The format is [origin]xpub.../**
        // We remove the last char in order to get the account_x_pub
        let key = AccountXPub::try_from(&key[..key.len() - 1])
            .expect("LedgerPolicy ensure correct format");
        key.descriptor_id()
    }
}
impl core::fmt::Display for LedgerPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl From<LedgerPolicy> for String {
    fn from(value: LedgerPolicy) -> Self {
        value.to_string()
    }
}
impl TryFrom<&str> for LedgerPolicy {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        // If the descriptor is not a tr, not OK for us
        let Some(caps) = re_descriptor().captures(value) else {
            return Err(Error::LedgerIncompatibleDescriptor(
                "not a Taproot descriptor",
            ));
        };
        let desc = &caps["desc"];
        let main_key = &caps["key"];
        let scripts = &caps["scripts"];
        // Main key must be an account xpub
        if !re_account_xpub().is_match(main_key) {
            return Err(Error::LedgerIncompatibleDescriptor(
                "Invalid key types in the descriptor, only Heritage account xpubs accepted",
            ));
        }
        // All keys in the scripts must be account xpubs
        if !re_pk()
            .captures_iter(scripts)
            .all(|cap| re_account_xpub().is_match(&cap["key"]))
        {
            return Err(Error::LedgerIncompatibleDescriptor(
                "Invalid key types in the descriptor, only Heritage account xpubs accepted",
            ));
        }
        // Ok we can consider it valid
        Ok(LedgerPolicy(
            re_account_xpub()
                .replace_all(desc, "${key}/**")
                .into_owned(),
        ))
    }
}

impl TryFrom<String> for LedgerPolicy {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        LedgerPolicy::try_from(value.as_str())
    }
}

impl From<&LedgerPolicy> for WalletPolicy {
    fn from(value: &LedgerPolicy) -> Self {
        let descriptor = &value.0;
        log::debug!("descriptor={descriptor}");
        let mut descriptor_template = descriptor.clone();
        let mut keys: Vec<WalletPubKey> = Vec::new();
        for account_xpub in re_account_xpub().captures_iter(descriptor) {
            log::debug!("account_xpub={account_xpub:?}");
            let key = &account_xpub["key"];
            log::debug!("key={key}");
            let pubkey = WalletPubKey::from_str(key).expect("xpub format is correct");
            let desc_index = if let Some(i) = keys.iter().position(|e| e == &pubkey) {
                i
            } else {
                keys.push(pubkey);
                keys.len() - 1
            };

            log::debug!("replace={} by @{}/**", &account_xpub[0], desc_index);

            descriptor_template =
                descriptor_template.replace(&account_xpub[0], &format!("@{}/**", desc_index));
        }

        log::debug!("descriptor_template={descriptor_template}");
        Self {
            name: "Heritage".to_owned(),
            version: ledger_bitcoin_client::wallet::Version::V2,
            descriptor_template,
            keys,
            threshold: None,
        }
    }
}
impl From<LedgerPolicy> for WalletPolicy {
    fn from(value: LedgerPolicy) -> Self {
        WalletPolicy::from(&value)
    }
}

impl TryFrom<SubwalletDescriptorBackup> for LedgerPolicy {
    type Error = Error;

    fn try_from(value: SubwalletDescriptorBackup) -> Result<Self, Self::Error> {
        let external_descriptor = value.external_descriptor.to_string();
        let change_descriptor = value.change_descriptor.to_string();
        if !re_account_xpub()
            .captures_iter(&external_descriptor)
            .zip(re_account_xpub().captures_iter(&change_descriptor))
            .all(|(k1, k2)| &k1["key"] == &k2["key"])
        {
            return Err(Error::LedgerIncompatibleDescriptor(
                "external and change descriptor templates would be different",
            ));
        }
        LedgerPolicy::try_from(external_descriptor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_valid_backup() {
        let valid_backup = r#"{
            "external_descriptor": "tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(12960),after(1731536000))))",
            "change_descriptor": "tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(12960),after(1731536000))))"
        }"#;
        let valid_backup: SubwalletDescriptorBackup = serde_json::from_str(valid_backup).unwrap();
        assert!(LedgerPolicy::try_from(valid_backup).is_ok())
    }

    #[test]
    fn from_invalid_backup() {
        let invalid_backup = r#"{
        "external_descriptor": "tr([44990794/86'/1'/0']tpubDE9DbziEKzUWbomb29YUwersoSERpmogW115aoegGezrf2uKJZfTqNCD5it8u8AzAuDUoCBcGgmwKppcFSEJ4fuLvBTDLsm5hmeK6L7LZcz/0/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1706602192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1722154192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1737706192)))}})",
        "change_descriptor": "tr([44990794/86'/1'/1']tpubDE9DbziEKzUWdSo28yKWmuEcgaXEF6tP11EB39RiZN5DW5XCEXRhWbMVRBsPv7yuWHBuueuN7WAhQ3kbEdvg4uMfCvwEYd8ay344UtfsWtz/1/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1706602192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1722154192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1737706192)))}})"
    }"#;
        let invalid_backup: SubwalletDescriptorBackup =
            serde_json::from_str(invalid_backup).unwrap();
        assert!(
            LedgerPolicy::try_from(invalid_backup).is_err_and(|e| match e {
                Error::LedgerIncompatibleDescriptor(msg) =>
                    msg == "external and change descriptor templates would be different",
                _ => unreachable!("Only LedgerIncompatibleDescriptor errors can be raised"),
            })
        )
    }

    #[test]
    fn valid_descriptor_1() {
        let valid_descriptor = r"tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1731536000))))";
        assert!(LedgerPolicy::try_from(valid_descriptor).is_ok())
    }
    #[test]
    fn invalid_descriptor_1() {
        let invalid_descriptor_1 = r"wsh([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/*";
        assert!(
            LedgerPolicy::try_from(invalid_descriptor_1).is_err_and(|e| match e {
                Error::LedgerIncompatibleDescriptor(msg) => msg == "not a Taproot descriptor",
                _ => unreachable!("Only LedgerIncompatibleDescriptor errors can be raised"),
            })
        )
    }
    #[test]
    fn invalid_descriptor_2() {
        let invalid_descriptor_2 = r"tr([44990794/86'/1'/0'/0]tpubDE9DbziEKzUWbomb29YUwersoSERpmogW115aoegGezrf2uKJZfTqNCD5it8u8AzAuDUoCBcGgmwKppcFSEJ4fuLvBTDLsm5hmeK6L7LZcz/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1706602192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1722154192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1737706192)))}})";
        assert!(
            LedgerPolicy::try_from(invalid_descriptor_2).is_err_and(|e| match e {
                Error::LedgerIncompatibleDescriptor(msg) => msg
                    == "Invalid key types in the descriptor, only Heritage account xpubs accepted",
                _ => unreachable!("Only LedgerIncompatibleDescriptor errors can be raised"),
            })
        )
    }
}
