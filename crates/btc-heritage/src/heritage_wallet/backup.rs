use std::collections::HashSet;

use crate::errors::Error;
use crate::miniscript::{Descriptor, DescriptorPublicKey};

use crate::bitcoin::bip32::Fingerprint;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
impl SubwalletDescriptorBackup {
    /// Return the [Fingerprint] of this [SubwalletDescriptorBackup]
    ///
    /// # Error
    /// Return an error if it is not the same on the `external_descriptor` and `change_descriptor`,
    /// or if they are not both Tr
    pub fn fingerprint(&self) -> Result<Fingerprint, Error> {
        let Descriptor::Tr(tr_ext) = &self.external_descriptor else {
            return Err(Error::InvalidBackup("external descriptor not Tr"));
        };
        let Descriptor::Tr(tr_change) = &self.change_descriptor else {
            return Err(Error::InvalidBackup("change descriptor not Tr"));
        };
        let fingerprint = tr_ext.internal_key().master_fingerprint();
        if fingerprint != tr_change.internal_key().master_fingerprint() {
            return Err(Error::InvalidBackup(
                "ext and change descriptors have different keys",
            ));
        }
        Ok(fingerprint)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct HeritageWalletBackup(pub(super) Vec<SubwalletDescriptorBackup>);
impl IntoIterator for HeritageWalletBackup {
    type Item = SubwalletDescriptorBackup;
    type IntoIter = <Vec<SubwalletDescriptorBackup> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
impl<'a> IntoIterator for &'a HeritageWalletBackup {
    type Item = &'a SubwalletDescriptorBackup;
    type IntoIter = <&'a Vec<SubwalletDescriptorBackup> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        (&self.0).into_iter()
    }
}
impl HeritageWalletBackup {
    /// Return the [Fingerprint] of this [HeritageWalletBackup]
    /// If there are not [SubwalletDescriptorBackup], return [Option::None]
    ///
    /// # Error
    /// Return an error if every [SubwalletDescriptorBackup] do not avec the same [Fingerprint]
    /// or if [SubwalletDescriptorBackup::fingerprint] returned an error.
    pub fn fingerprint(&self) -> Result<Option<Fingerprint>, Error> {
        let h_fingerprint = self
            .0
            .iter()
            .map(|sdb| sdb.fingerprint())
            .collect::<Result<HashSet<_>, _>>()?;
        if h_fingerprint.len() > 1 {
            return Err(Error::InvalidBackup("multiple fingerprint in the backup"));
        }
        Ok(h_fingerprint.into_iter().next())
    }

    pub fn iter(&self) -> core::slice::Iter<'_, SubwalletDescriptorBackup> {
        self.into_iter()
    }
}
