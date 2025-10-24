use std::collections::HashSet;

use crate::errors::Error;
use crate::miniscript::{Descriptor, DescriptorPublicKey};

use crate::bitcoin::bip32::Fingerprint;
use serde::{Deserialize, Serialize};

/// Backup information for a single subwallet configuration
///
/// Contains all the necessary information to restore a subwallet, including
/// its descriptors and usage state. This allows wallet history reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubwalletDescriptorBackup {
    /// External (receiving) address descriptor for the subwallet
    pub external_descriptor: Descriptor<DescriptorPublicKey>,
    /// Internal (change) address descriptor for the subwallet
    pub change_descriptor: Descriptor<DescriptorPublicKey>,
    /// Unix timestamp of first usage, if the subwallet was ever used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_use_ts: Option<u64>,
    /// Last used external address index, if any addresses were generated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_external_index: Option<u32>,
    /// Last used change address index, if any change addresses were generated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change_index: Option<u32>,
}
impl SubwalletDescriptorBackup {
    /// Returns the master key [Fingerprint] for this subwallet backup
    ///
    /// Extracts and validates the master key fingerprint from both descriptors.
    /// Both external and change descriptors must be Taproot descriptors with
    /// the same master key fingerprint.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Either descriptor is not a Taproot descriptor
    /// - The fingerprints from external and change descriptors don't match
    /// - The descriptors are malformed
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

/// Complete backup of a heritage wallet
///
/// Contains backup information for all subwallet configurations (both current
/// and obsolete) that make up a complete heritage wallet. This enables full
/// wallet restoration including transaction history and address derivation state.
///
/// The backup maintains chronological order of subwallet configurations to
/// preserve the wallet's evolution over time.
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
    /// Returns the master key [Fingerprint] for this wallet backup
    ///
    /// Validates that all subwallet backups share the same master key fingerprint,
    /// which ensures backup consistency. Returns `None` if the backup is empty.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Subwallets have different master key fingerprints (inconsistent backup)
    /// - Any subwallet backup has invalid fingerprint
    /// - Descriptor parsing fails for any subwallet
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

    /// Returns an iterator over the subwallet descriptor backups
    ///
    /// Provides access to individual subwallet backups contained within
    /// this heritage wallet backup.
    pub fn iter(&self) -> core::slice::Iter<'_, SubwalletDescriptorBackup> {
        self.into_iter()
    }
}
