use std::{fmt::Display, str::FromStr};

use bdk::{
    bitcoin::{bip32::ChildNumber, Network},
    miniscript::DescriptorPublicKey,
};
use serde::{Deserialize, Serialize};

use crate::{errors::Error, utils};

pub type AccountXPubId = u32;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountXPub(DescriptorPublicKey);

impl Serialize for AccountXPub {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for AccountXPub {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AccountXPubVisitor;
        impl<'de> serde::de::Visitor<'de> for AccountXPubVisitor {
            type Value = AccountXPub;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a DescriptorPublicKey string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(AccountXPub::try_from(value).map_err(|e| E::custom(e.to_string()))?)
            }
        }
        deserializer.deserialize_str(AccountXPubVisitor)
    }
}

impl AccountXPub {
    /// Return the ID, which is the last number from the [bdk::bitcoin::bip32::DerivationPath] (the hardened account_id)
    pub fn descriptor_id(&self) -> AccountXPubId {
        let derivation_path = self
            .0
            .full_derivation_path()
            .expect("multipath extended keys not supported");
        let last_child = derivation_path[2];
        if let ChildNumber::Hardened { index } = last_child {
            index
        } else {
            panic!("AccountXPub DerivationPath is unexpected ({derivation_path})")
        }
    }

    pub fn descriptor_public_key(&self) -> &DescriptorPublicKey {
        &self.0
    }
}

impl Display for AccountXPub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<DescriptorPublicKey> for AccountXPub {
    type Error = Error;

    fn try_from(descriptor: DescriptorPublicKey) -> Result<Self, Self::Error> {
        // If the DescriptorPublicKey is not XPub, bail
        if let DescriptorPublicKey::XPub(xpub) = &descriptor {
            xpub.origin
                .as_ref()
                .ok_or(Error::InvalidDescriptorPublicKey(
                    "XPub must have origin information",
                ))?;
        } else {
            return Err(Error::InvalidDescriptorPublicKey(
                "Must be a DescriptorPublicKey::XPub variant",
            ));
        };

        // If the derivation path is not m/86'/[0,1]'/i'/*, bail
        let cointype_path_segment = match utils::bitcoin_network_from_env() {
            Network::Bitcoin => 0,
            _ => 1,
        };
        let derivation_path = descriptor
            .full_derivation_path()
            .expect("descriptor has been verified to be an XPub");
        if !(derivation_path.len() == 3
            && derivation_path[..2]
                == [
                    ChildNumber::from_hardened_idx(86).expect("86 is in boundaries"),
                    ChildNumber::from_hardened_idx(cointype_path_segment)
                        .expect("0 and 1 are in boundaries"),
                ]
            && derivation_path[2].is_hardened()
            && descriptor.has_wildcard())
        {
            log::error!("DescriptorPublicKey must have a Derivation Path like m/86'/{cointype_path_segment}'/<account>'/*");
            return Err(Error::InvalidDescriptorPublicKey("Wrong derivation path"));
        }

        Ok(Self(descriptor))
    }
}

impl TryFrom<&str> for AccountXPub {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let descriptor = DescriptorPublicKey::from_str(value).map_err(|e| {
            log::error!("Error parsing DescriptorPublicKey string: {e:#}");
            Error::InvalidDescriptorPublicKey("Parse error")
        })?;
        AccountXPub::try_from(descriptor)
    }
}

impl TryFrom<String> for AccountXPub {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        AccountXPub::try_from(value.as_str())
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::tests::get_test_account_xpub;

    #[test]
    fn accepted_account_xpub() {
        // Correct AccountXPub
        assert!(AccountXPub::try_from("[73c5da0a/86'/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_ok());

        // Not an xpub
        assert!(AccountXPub::try_from("[3f635a63/86'/1'/1751476594'/0/0]03d95a176f14da8363caaa196a3b94b0b53bd9601bdb9221bd85dceb6b501e5822").is_err());
        // No origin info
        assert!(AccountXPub::try_from("tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // No derivation path
        assert!(AccountXPub::try_from("[73c5da0a]tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // No fingerprint
        assert!(AccountXPub::try_from("[m/86'/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Derivation path too short
        assert!(AccountXPub::try_from("[73c5da0a/86'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Derivation path too long
        assert!(AccountXPub::try_from("[73c5da0a/86'/0'/0'/0]tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Network wrong
        assert!(AccountXPub::try_from("[73c5da0a/86'/0'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Usage not hardened
        assert!(AccountXPub::try_from("[73c5da0a/86/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Incorrect usage
        assert!(AccountXPub::try_from("[73c5da0a/87'/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Incorrect master pub key
        assert!(AccountXPub::try_from("[73c5da0a/86'/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH4u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // Not extensible (no wildcard)
        assert!(AccountXPub::try_from("[73c5da0a/86'/1'/0']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX").is_err());
    }

    #[test]
    fn account_xpub_id() {
        for i in 0..20 {
            assert_eq!(get_test_account_xpub(i).descriptor_id(), i);
        }
    }
}
