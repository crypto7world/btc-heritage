use core::fmt::Display;

use serde::{Deserialize, Serialize};

use crate::{
    bitcoin::{
        bip32::{ChildNumber, DerivationPath},
        Network,
    },
    errors::Error,
    miniscript::{
        descriptor::{DescriptorXKey, Wildcard},
        DescriptorPublicKey,
    },
    utils,
};

/// Unique identifier for an AccountXPub, derived from the hardened account index of the Derivation Path
pub type AccountXPubId = u32;

/// A BIP86 account extended public key for Taproot addresses
///
/// This struct wraps a [`DescriptorPublicKey`] and enforces that it follows the BIP86
/// derivation path format: `m/86'/{coin_type}'/{account}'/*` where `coin_type` is 0
/// for mainnet and 1 for testnet/regtest/signet.
///
/// The AccountXPub can be used to derive child keys for external and change key chains
/// and ultimately generating Taproot addresses.
#[derive(Debug, Clone, Hash, Serialize, Eq, PartialEq, Ord, PartialOrd)]
#[serde(into = "String")]
pub struct AccountXPub(DescriptorPublicKey);

impl<'de> Deserialize<'de> for AccountXPub {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AccountXPubVisitor;
        impl<'de> serde::de::Visitor<'de> for AccountXPubVisitor {
            type Value = AccountXPub;

            fn expecting(&self, formatter: &mut core::fmt::Formatter) -> core::fmt::Result {
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
    /// Returns the account ID extracted from the derivation path
    ///
    /// The ID is the hardened account index from the BIP86 derivation path
    /// `m/86'/{coin_type}'/{account}'/*`. For example, an AccountXPub with
    /// derivation path `m/86'/1'/5'/*` would return `5`.
    ///
    /// # Panics
    ///
    /// Panics if the derivation path doesn't match the expected BIP86 format
    /// or if multipath extended keys are used (not supported).
    ///
    /// # Example
    ///
    /// ```
    /// # use btc_heritage::utils::bitcoin_network;
    /// # use btc_heritage::bitcoin;
    /// # use bitcoin::Network;
    /// // Set the Network to Testnet
    /// bitcoin_network::set(Network::Testnet);
    ///
    /// // Create an account xpub
    /// use btc_heritage::AccountXPub;
    /// let axpub = AccountXPub::try_from("[73c5da0a/86'/1'/12']tpubDDfvzhdVV4unsoKt5aE6dcsNsfeWbTgmLZPi8LQDYU2xixrYemMfWJ3BaVneH3u7DBQePdTwhpybaKRU95pi6PMUtLPBJLVQRpzEnjfjZzX/*").unwrap();
    ///
    /// // As the account is 12, that's what is returned
    /// assert_eq!(axpub.descriptor_id(), 12);
    /// ```
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

    /// Returns a reference to the underlying descriptor public key
    pub fn descriptor_public_key(&self) -> &DescriptorPublicKey {
        &self.0
    }

    /// Creates a child descriptor public key for the specified derivation index
    ///
    /// This generates a descriptor that can be used to derive addresses at the
    /// specified index within this account.
    pub fn child_descriptor_public_key(&self, index: u32) -> DescriptorPublicKey {
        log::debug!("AccountXPub::child_descriptor_public_key - index={index}");
        let (fingerprint, derivation_path, account_xpub_key) = match &self.0 {
            DescriptorPublicKey::XPub(DescriptorXKey {
                origin: Some((fingerprint, path)),
                xkey,
                ..
            }) => (fingerprint, path, xkey),
            _ => panic!(
                "Invalid key variant, should never happen as AccountXPub is checked at creation"
            ),
        };
        log::debug!("AccountXPub::child_descriptor_public_key - fingerprint={fingerprint}");
        log::debug!("AccountXPub::child_descriptor_public_key - derivation_path={derivation_path}");
        log::debug!(
            "AccountXPub::child_descriptor_public_key - account_xpub_key={account_xpub_key}"
        );
        let child_deriv_path = DerivationPath::from(vec![ChildNumber::from(index)]);
        DescriptorPublicKey::XPub(DescriptorXKey {
            origin: Some((*fingerprint, derivation_path.clone())),
            xkey: *account_xpub_key,
            derivation_path: child_deriv_path,
            wildcard: Wildcard::Unhardened,
        })
    }
}

impl Display for AccountXPub {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}
impl From<AccountXPub> for String {
    fn from(value: AccountXPub) -> Self {
        value.to_string()
    }
}

impl TryFrom<DescriptorPublicKey> for AccountXPub {
    type Error = Error;

    /// Converts a descriptor public key into an AccountXPub
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDescriptorPublicKey`] if:
    /// - The descriptor is not an XPub variant
    /// - The descriptor lacks origin information
    /// - The descriptor has a derivation path after the key
    /// - The derivation path doesn't follow BIP86 format `m/86'/{coin_type}'/{account}'/*`
    /// - The descriptor doesn't have a wildcard for address generation
    fn try_from(descriptor: DescriptorPublicKey) -> Result<Self, Self::Error> {
        // If the DescriptorPublicKey is not XPub, bail
        if let DescriptorPublicKey::XPub(xpub) = &descriptor {
            xpub.origin
                .as_ref()
                .ok_or(Error::InvalidDescriptorPublicKey(
                    "DescriptorPublicKey must have origin information",
                ))?;
            if !xpub.derivation_path.is_empty() {
                log::error!("DescriptorPublicKey must have no derivation path after the key");
                return Err(Error::InvalidDescriptorPublicKey(
                    "Derivation after the key",
                ));
            }
        } else {
            return Err(Error::InvalidDescriptorPublicKey(
                "Must be a DescriptorPublicKey::XPub variant",
            ));
        };

        // If the derivation path is not m/86'/[0,1]'/i'/*, bail
        let cointype_path_segment = match utils::bitcoin_network::get() {
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

impl core::str::FromStr for AccountXPub {
    type Err = Error;

    /// Parses a string into an AccountXPub
    ///
    /// The string should be a valid descriptor public key following BIP86 format.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDescriptorPublicKey`] if the string cannot be
    /// parsed as a valid descriptor public key or doesn't meet AccountXPub requirements.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let descriptor = s.parse::<DescriptorPublicKey>().map_err(|e| {
            log::error!("Error parsing DescriptorPublicKey string: {e:#}");
            Error::InvalidDescriptorPublicKey("Parse error")
        })?;
        AccountXPub::try_from(descriptor)
    }
}

impl TryFrom<&str> for AccountXPub {
    type Error = Error;

    /// Converts a string slice into an AccountXPub
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDescriptorPublicKey`] if the string cannot be
    /// parsed as a valid AccountXPub.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<String> for AccountXPub {
    type Error = Error;

    /// Converts a String into an AccountXPub
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDescriptorPublicKey`] if the string cannot be
    /// parsed as a valid AccountXPub.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
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
