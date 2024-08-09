use crate::miniscript::{Descriptor, DescriptorPublicKey};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
pub struct SubwalletDescriptorBackup {
    pub external_descriptor: Descriptor<DescriptorPublicKey>,
    pub change_descriptor: Descriptor<DescriptorPublicKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_use_ts: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_external_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(any(test, feature = "database-tests"), derive(Eq, PartialEq))]
pub struct HeritageWalletBackup(pub(super) Vec<SubwalletDescriptorBackup>);
impl IntoIterator for HeritageWalletBackup {
    type Item = SubwalletDescriptorBackup;
    type IntoIter = <Vec<SubwalletDescriptorBackup> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
