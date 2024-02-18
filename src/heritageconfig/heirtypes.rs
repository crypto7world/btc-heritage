use std::{fmt::Display, str::FromStr};

use bdk::miniscript::DescriptorPublicKey;
use serde::{Deserialize, Serialize};

#[derive(Debug, Hash, Clone, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(into = "String", from = "String")]
// TODO: Struct should perform checks on the value it receives???
pub struct SingleHeirPubkey(DescriptorPublicKey);
impl Into<String> for SingleHeirPubkey {
    fn into(self) -> String {
        self.0.to_string()
    }
}
impl From<&str> for SingleHeirPubkey {
    fn from(value: &str) -> Self {
        Self(DescriptorPublicKey::from_str(value).unwrap())
    }
}
impl From<String> for SingleHeirPubkey {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}
impl Display for SingleHeirPubkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Hash, Serialize, Deserialize, Eq, PartialEq, Ord, PartialOrd, Clone)]
#[serde(tag = "type", content = "value", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HeirConfig {
    SingleHeirPubkey(SingleHeirPubkey),
    // SingleHeirPubKeyHash(KeyHash),
}

impl HeirConfig {
    pub fn descriptor_segment(&self) -> String {
        match self {
            HeirConfig::SingleHeirPubkey(xpub) => format!("v:pk({xpub})"),
            // HeritageMode::SingleHeirPubKeyHash(pubkeyhash) => {
            //     let s: String = (*pubkeyhash).into();
            //     format!("vc:expr_raw_pkh({s})")
            // }
        }
    }
}
