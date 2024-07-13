use serde::{Deserialize, Serialize};
use std::{fmt::Display, str::FromStr};

use crate::{
    bitcoin::{
        bip32::{ChildNumber, DerivationPath, Fingerprint},
        Network,
    },
    errors::Error,
    miniscript::{DescriptorPublicKey, ToPublicKey},
    utils, AccountXPub,
};

#[derive(Debug, Hash, Clone, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(into = "String")]
pub struct SingleHeirPubkey(DescriptorPublicKey);

impl<'de> Deserialize<'de> for SingleHeirPubkey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct SingleHeirPubkeyVisitor;
        impl<'de> serde::de::Visitor<'de> for SingleHeirPubkeyVisitor {
            type Value = SingleHeirPubkey;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a DescriptorPublicKey string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(SingleHeirPubkey::try_from(value).map_err(|e| E::custom(e.to_string()))?)
            }
        }
        deserializer.deserialize_str(SingleHeirPubkeyVisitor)
    }
}
impl TryFrom<DescriptorPublicKey> for SingleHeirPubkey {
    type Error = Error;

    fn try_from(descriptor: DescriptorPublicKey) -> Result<Self, Self::Error> {
        // If the DescriptorPublicKey is not XPub, bail
        if let DescriptorPublicKey::Single(spub) = &descriptor {
            spub.origin
                .as_ref()
                .ok_or(Error::InvalidDescriptorPublicKey(
                    "DescriptorPublicKey must have origin information",
                ))?;
        } else {
            return Err(Error::InvalidDescriptorPublicKey(
                "Must be a DescriptorPublicKey::Single variant",
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
        if !(derivation_path.len() == 5
            && derivation_path[..2]
                == [
                    ChildNumber::from_hardened_idx(86).expect("86 is in boundaries"),
                    ChildNumber::from_hardened_idx(cointype_path_segment)
                        .expect("0 and 1 are in boundaries"),
                ]
            && derivation_path[2].is_hardened()
            && !descriptor.has_wildcard())
        {
            log::error!("DescriptorPublicKey must have a Derivation Path like m/86'/{cointype_path_segment}'/<account>'/<M>/<N>");
            return Err(Error::InvalidDescriptorPublicKey("Wrong derivation path"));
        }

        Ok(Self(descriptor))
    }
}

impl TryFrom<&str> for SingleHeirPubkey {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let descriptor = DescriptorPublicKey::from_str(value).map_err(|e| {
            log::error!("Error parsing DescriptorPublicKey string: {e:#}");
            Error::InvalidDescriptorPublicKey("Parse error")
        })?;
        SingleHeirPubkey::try_from(descriptor)
    }
}

impl TryFrom<String> for SingleHeirPubkey {
    type Error = Error;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        SingleHeirPubkey::try_from(value.as_str())
    }
}

impl From<SingleHeirPubkey> for String {
    fn from(value: SingleHeirPubkey) -> Self {
        value.to_string()
    }
}

impl Display for SingleHeirPubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Clone)]
#[serde(tag = "type", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HeirConfig {
    SingleHeirPubkey(SingleHeirPubkey),
    HeirXPubkey(AccountXPub),
    // SingleHeirPubKeyHash(KeyHash),
}

impl core::hash::Hash for HeirConfig {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        match self {
            // We want to conserve the historic Hash implementation
            // from when there was only one HeirConfig variant
            // for retro-compatibility so we simply hash the encapsulated value
            HeirConfig::SingleHeirPubkey(shp) => shp.hash(state),
            // All additional variants will be hashed with a prefix
            HeirConfig::HeirXPubkey(hxp) => {
                "hxp".hash(state);
                hxp.hash(state)
            }
        };
    }
}

impl HeirConfig {
    pub fn descriptor_segment(&self, xpub_child_index: Option<u32>) -> String {
        match self {
            HeirConfig::SingleHeirPubkey(xpub) => format!("v:pk({xpub})"),
            HeirConfig::HeirXPubkey(xpub) => match xpub_child_index {
                Some(index) => format!("v:pk({})", xpub.child_descriptor_public_key(index)),
                None => format!("v:pk({})", xpub),
            }, // HeritageMode::SingleHeirPubKeyHash(pubkeyhash) => {
               //     let s: String = (*pubkeyhash).into();
               //     format!("vc:expr_raw_pkh({s})")
               // }
        }
    }
    pub fn concrete_script_segment<'a>(
        &self,
        origins: impl Iterator<Item = (&'a Fingerprint, &'a DerivationPath)>,
    ) -> String {
        match self {
            HeirConfig::SingleHeirPubkey(xpub) => format!("v:pk({xpub})"),
            HeirConfig::HeirXPubkey(xpub) => {
                let fingerprint = xpub.descriptor_public_key().master_fingerprint();
                let derivation_path = xpub
                    .descriptor_public_key()
                    .full_derivation_path()
                    .expect("account Xpub has a derivation path");
                let mut origins = origins.filter_map(|(f, d)| {
                    if *f == fingerprint && d[..3] == derivation_path[..] {
                        let chain = d[3];
                        let address_index = d[4];
                        Some(
                            xpub.child_descriptor_public_key(chain.into())
                                .at_derivation_index(address_index.into())
                                .unwrap()
                                .to_x_only_pubkey(),
                        )
                    } else {
                        None
                    }
                });
                let key = origins
                    .next()
                    .expect("Caller should gave us an origin that covers the Xpub");
                if origins.next().is_some() {
                    panic!("Having multiple origins candidates is unexpected");
                }
                format!("v:pk({key})")
            } // HeritageMode::SingleHeirPubKeyHash(pubkeyhash) => {
              //     let s: String = (*pubkeyhash).into();
              //     format!("vc:expr_raw_pkh({s})")
              // }
        }
    }
    pub fn has_fingerprint(&self, fingerprint: Fingerprint) -> bool {
        match self {
            HeirConfig::SingleHeirPubkey(xpub) => xpub.0.master_fingerprint() == fingerprint,
            HeirConfig::HeirXPubkey(xpub) => {
                xpub.descriptor_public_key().master_fingerprint() == fingerprint
            } // HeritageMode::SingleHeirPubKeyHash(pubkeyhash) => {
              //     let s: String = (*pubkeyhash).into();
              //     format!("vc:expr_raw_pkh({s})")
              // }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn accepted_single_heir_pubkey() {
        // Correct SingleHeirPubkey
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_ok());

        // Not a simple pub key
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86'/1'/1751476594'/0/0]tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").is_err());
        // No origin info
        assert!(SingleHeirPubkey::try_from(
            "02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b"
        )
        .is_err());
        // No derivation path
        assert!(SingleHeirPubkey::try_from(
            "[99ccb69a]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b"
        )
        .is_err());
        // No fingerprint
        assert!(SingleHeirPubkey::try_from("[m/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
        // Derivation path too short
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86'/1'/1751476594'/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
        // Derivation path too long
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86'/1'/1751476594'/0/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
        // Network wrong
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86'/0'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
        // Usage not hardened
        assert!(SingleHeirPubkey::try_from("[99ccb69a/86/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
        // Incorrect usage
        assert!(SingleHeirPubkey::try_from("[99ccb69a/87'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b").is_err());
    }
}
