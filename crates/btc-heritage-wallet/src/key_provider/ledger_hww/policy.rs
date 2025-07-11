//! Ledger Hardware Wallet Policy Support
//!
//! This module provides support for converting Bitcoin Heritage wallet descriptors
//! into Ledger-compatible wallet policies as defined in BIP-388. The Ledger device
//! uses a simplified policy format that reduces descriptor size and enables secure
//! validation on hardware with limited resources.
//!
//! ## Key Concepts
//!
//! - **Wallet Policy**: A BIP-388 compliant policy that represents a Bitcoin descriptor
//!   in a compact format suitable for Ledger devices
//! - **Heritage Account XPub**: Extended public keys with specific derivation paths
//!   following the pattern `m/86'/[0,1]'/<account>'/*` for Taproot wallets
//! - **Policy Template**: A descriptor template where xpubs are replaced with placeholders
//!   like `@0/**`, `@1/**`, etc.
//!
//! ## Ledger Policy Format
//!
//! The Ledger policy format differs from standard Bitcoin descriptors in several ways:
//! - Uses `/**` for both external and change derivation paths instead of separate descriptors
//! - Replaces actual xpubs with indexed placeholders (`@0`, `@1`, etc.)
//! - Only supports Taproot descriptors (`tr(...)`)
//! - Requires all keys to be Heritage account xpubs with proper derivation paths
//!
//! ## Example
//!
//! A standard Heritage descriptor:
//! ```text
//! tr([origin/86'/1'/0']xpub.../0/*,scripts...)
//! ```
//!
//! Becomes a Ledger policy template:
//! ```text
//! tr(@0/**,scripts...)
//! ```
//!
//! With the xpub stored separately in the key information vector.

use core::str::FromStr;

use bitcoin::hex::{Case, DisplayHex, FromHex};
use btc_heritage::{AccountXPub, AccountXPubId, SubwalletDescriptorBackup};
use ledger_bitcoin_client::{WalletPolicy, WalletPubKey};
use serde::{Deserialize, Serialize};

use crate::errors::Error;

/// Returns a regex for parsing Taproot descriptors
///
/// This regex matches the overall structure of a Taproot descriptor:
/// - `tr(key)` - Simple key-path-only Taproot
/// - `tr(key,scripts)` - Taproot with script paths
/// - Optional descriptor checksum suffix `#[a-z0-9]{8}`
///
/// Named capture groups:
/// - `desc`: The main descriptor part without checksum
/// - `key`: The internal key (first parameter)
/// - `scripts`: The script tree (second parameter, optional)
fn re_descriptor() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"^(?<desc>tr\((?<key>.+?)(?:,(?<scripts>.+))?\))(:?#[a-z0-9]{8})?$")
            .unwrap()
    })
}

/// Returns a regex for parsing public key expressions in script paths
///
/// This regex matches various public key wrapper functions:
/// - `pk(key)` - Raw public key
/// - `pkh(key)` - Public key hash
/// - `pk_k(key)` - Public key with specific encoding
/// - `pk_h(key)` - Public key hash with specific encoding
///
/// Named capture groups:
/// - `key`: The key expression inside the wrapper
fn re_pk() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(:?pkh?|pk_[kh])\((?<key>.+?)\)").unwrap())
}

/// Returns a regex for parsing Heritage account extended public keys
///
/// This regex matches Heritage account xpubs with the following constraints:
/// - Must have origin information: `[fingerprint/86'/cointype'/account']`
/// - Must use purpose 86 (Taproot) in the derivation path
/// - Must use cointype 0 (mainnet) or 1 (testnet)
/// - Must have hardened account index
/// - Must be followed by a derivation pattern
///
/// Named capture groups:
/// - `key`: The complete xpub with origin info (without final derivation)
/// - `derivation`: The derivation pattern (`/**` or `(/num)*/`)
///
/// The `/**` pattern is Ledger-specific and represents both external (0) and change (1) paths,
/// equivalent to `/<0;1>/*` in BIP-388 multipath descriptors.
fn re_account_xpub() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r"(?<key>\[[0-9a-f]{8}\/86['h]\/[01]['h]\/[0-9]+['h]\][tx]pub[1-9A-HJ-NP-Za-km-z]{79,108})(?<derivation>\/\*\*|(:?\/[0-9]+)*\/\*)",
        )
        .unwrap()
    })
}

/// Macro to generate 32-byte wrapper types for Ledger policy identifiers
///
/// This macro creates a new type that wraps a 32-byte array and provides:
/// - Serialization/deserialization as hex strings
/// - Display formatting as lowercase hex
/// - Conversions to/from hex strings and byte arrays
/// - Debug and Clone implementations
///
/// These types are used for Ledger-specific identifiers that must be exactly 32 bytes.
macro_rules! new_byte_type {
    ($(#[$comments:meta])* $struct_name:ident) => {
        #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
        #[serde(into = "String", try_from = "String")]
        $(#[$comments])*
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
new_byte_type! {
    #[doc = "A 32-byte identifier for a Ledger wallet policy"]
    #[doc = ""]
    #[doc = "This is used by Ledger devices to uniquely identify registered wallet policies."]
    #[doc = "The policy ID is typically derived from the policy content and ensures that"]
    #[doc = "the same policy will always have the same ID."]
    LedgerPolicyId
}
new_byte_type! {
    #[doc = "A 32-byte HMAC for a Ledger wallet policy"]
    #[doc = ""]
    #[doc = "This serves as a \"proof of registration\" for stateless Ledger devices."]
    #[doc = "The HMAC is computed over the policy content using a device-specific key,"]
    #[doc = "allowing the device to verify that a policy was previously approved by the user"]
    #[doc = "without needing to store the policy permanently."]
    LedgerPolicyHMAC
}

/// A Ledger-compatible wallet policy representation
///
/// This struct encapsulates a BIP-388 compatible wallet policy that can be used
/// with Ledger hardware wallets. It validates that the policy meets Ledger's
/// requirements and converts Heritage descriptors into the appropriate format.
///
/// # Format Requirements
///
/// - Must be a Taproot descriptor starting with `tr(`
/// - All keys must be Heritage account xpubs with proper derivation paths
/// - Uses `/**` derivation pattern instead of separate external/change descriptors
/// - Replaces actual xpubs with indexed placeholders (`@0/**`, `@1/**`, etc.)
///
/// # Example
///
/// ```text
/// // Original Heritage descriptor:
/// tr([origin/86'/1'/0']xpub.../0/*,and_v(v:pk([origin2/86'/1'/1']xpub2.../0/*),older(144)))
///
/// // Becomes Ledger policy:
/// tr(@0/**,and_v(v:pk(@1/**),older(144)))
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(into = "String", try_from = "String")]
pub struct LedgerPolicy(String);

impl LedgerPolicy {
    /// Returns the account ID for the primary account xpub in this policy
    ///
    /// This method extracts the first Heritage account xpub from the policy
    /// and returns its account ID, which is the hardened account index from
    /// the derivation path (e.g., for `m/86'/1'/5'`, the account ID is 5).
    ///
    /// # Returns
    ///
    /// The account ID of the primary account xpub used in this policy.
    ///
    /// # Panics
    ///
    /// Panics if the policy doesn't contain a valid Heritage account xpub.
    /// This should never happen for a properly validated `LedgerPolicy`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use btc_heritage_wallet::LedgerPolicy;
    /// let policy_str = "tr([origin/86'/1'/42']xpub.../*)";
    /// let policy = LedgerPolicy::try_from(policy_str).unwrap();
    /// assert_eq!(policy.get_account_id(), 42);
    /// ```
    pub fn get_account_id(&self) -> AccountXPubId {
        let key = re_account_xpub()
            .find(&self.0)
            .expect("LedgerPolicy ensure the descriptor contains an account_xpub")
            .as_str();
        // The format is [origin]xpub.../**
        // We remove the last character (the final '*') to get the account_xpub
        // This converts from "[origin]xpub.../**" to "[origin]xpub.../*"
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
        value.0
    }
}

impl TryFrom<&str> for LedgerPolicy {
    type Error = Error;

    /// Creates a Ledger policy from a Bitcoin descriptor string
    ///
    /// This method validates that the descriptor is compatible with Ledger devices
    /// and converts it to the appropriate policy format.
    ///
    /// # Validation Rules
    ///
    /// 1. Must be a Taproot descriptor (starts with `tr(`)
    /// 2. The internal key must be a Heritage account xpub
    /// 3. All keys in script paths must be Heritage account xpubs
    /// 4. Heritage account xpubs must follow the pattern `[fingerprint/86'/cointype'/account']xpub.../derivation`
    ///
    /// # Conversion Process
    ///
    /// The method converts Heritage descriptors to Ledger format by:
    /// 1. Replacing all account xpubs with `@index/**` placeholders
    /// 2. Ensuring the `/**` derivation pattern is used consistently
    /// 3. Preserving the script structure while simplifying key references
    ///
    /// # Arguments
    ///
    /// * `value` - A Bitcoin descriptor string to convert
    ///
    /// # Returns
    ///
    /// A `LedgerPolicy` if the descriptor is valid and compatible.
    ///
    /// # Errors
    ///
    /// Returns `Error::LedgerIncompatibleDescriptor` if:
    /// - The descriptor is not a Taproot descriptor
    /// - Any key is not a Heritage account xpub
    /// - The descriptor structure is invalid
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use btc_heritage_wallet::LedgerPolicy;
    /// let descriptor = "tr([origin/86'/1'/0']xpub.../0/*,and_v(v:pk([origin2/86'/1'/1']xpub2.../0/*),older(144)))";
    /// let policy = LedgerPolicy::try_from(descriptor).unwrap();
    /// // Results in: "tr(@0/**,and_v(v:pk(@1/**),older(144)))"
    /// ```
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        // Parse the descriptor structure - must be a Taproot descriptor
        let Some(caps) = re_descriptor().captures(value) else {
            return Err(Error::LedgerIncompatibleDescriptor(
                "not a Taproot descriptor",
            ));
        };
        let desc = &caps["desc"];
        let main_key = &caps["key"];
        let scripts = &caps["scripts"];

        // Validate that the main/internal key is a Heritage account xpub
        if !re_account_xpub().is_match(main_key) {
            return Err(Error::LedgerIncompatibleDescriptor(
                "Invalid key types in the descriptor, only Heritage account xpubs accepted",
            ));
        }

        // Validate that all keys in script paths are Heritage account xpubs
        // This iterates through all pk(), pkh(), etc. expressions in the scripts
        if !re_pk()
            .captures_iter(scripts)
            .all(|cap| re_account_xpub().is_match(&cap["key"]))
        {
            return Err(Error::LedgerIncompatibleDescriptor(
                "Invalid key types in the descriptor, only Heritage account xpubs accepted",
            ));
        }

        // Convert to Ledger policy format by replacing all account xpubs with "/**"
        // The regex replacement uses the captured "key" group and appends "/**"
        // This transforms "[origin]xpub.../derivation" to "[origin]xpub.../**"
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
    /// Converts a LedgerPolicy to a Ledger WalletPolicy
    ///
    /// This method transforms the internal policy format into the specific
    /// `WalletPolicy` structure required by the Ledger Bitcoin client library.
    ///
    /// # Conversion Process
    ///
    /// 1. Extracts all unique Heritage account xpubs from the policy
    /// 2. Builds a key information vector with `WalletPubKey` instances
    /// 3. Replaces xpub references with indexed placeholders (`@0/**`, `@1/**`, etc.)
    /// 4. Creates a BIP-388 compatible descriptor template
    /// 5. Assembles the final `WalletPolicy` structure
    ///
    /// # Key Deduplication
    ///
    /// If the same xpub appears multiple times in the policy, it's only included
    /// once in the keys vector, and all references point to the same index.
    ///
    /// # Arguments
    ///
    /// * `value` - A reference to the `LedgerPolicy` to convert
    ///
    /// # Returns
    ///
    /// A `WalletPolicy` ready for use with Ledger devices.
    ///
    /// # Panics
    ///
    /// Panics if the policy contains invalid xpub formats. This should never
    /// happen for a properly validated `LedgerPolicy`.
    fn from(value: &LedgerPolicy) -> Self {
        let account_id = value.get_account_id();
        let descriptor = &value.0;
        log::debug!("descriptor={descriptor}");
        let mut descriptor_template = descriptor.clone();
        let mut keys: Vec<WalletPubKey> = Vec::new();

        // Process each account xpub found in the descriptor
        for account_xpub in re_account_xpub().captures_iter(descriptor) {
            log::debug!("account_xpub={account_xpub:?}");
            let key = &account_xpub["key"];
            log::debug!("key={key}");

            // Parse the xpub into a WalletPubKey for the Ledger client
            let pubkey = WalletPubKey::from_str(key).expect("xpub format is correct");

            // Check if we've already seen this xpub to avoid duplicates
            let desc_index = if let Some(i) = keys.iter().position(|e| e == &pubkey) {
                i // Reuse existing index
            } else {
                keys.push(pubkey);
                keys.len() - 1 // Use new index
            };

            log::debug!("replace={} by @{}/**", &account_xpub[0], desc_index);

            // Replace the full xpub expression with a placeholder
            // This converts "[origin]xpub.../**" to "@index/**"
            descriptor_template =
                descriptor_template.replace(&account_xpub[0], &format!("@{}/**", desc_index));
        }

        log::debug!("descriptor_template={descriptor_template}");
        Self {
            name: format!("Heritage #{account_id}"),
            version: ledger_bitcoin_client::wallet::Version::V2,
            descriptor_template,
            keys,
            threshold: None, // Heritage policies don't use simple thresholds
        }
    }
}

impl From<LedgerPolicy> for WalletPolicy {
    fn from(value: LedgerPolicy) -> Self {
        WalletPolicy::from(&value)
    }
}

impl TryFrom<&SubwalletDescriptorBackup> for LedgerPolicy {
    type Error = Error;

    /// Creates a Ledger policy from a subwallet descriptor backup
    ///
    /// This method converts a Heritage subwallet backup (which contains separate
    /// external and change descriptors) into a unified Ledger policy that uses
    /// the `/**` derivation pattern.
    ///
    /// # Validation
    ///
    /// The method ensures that both external and change descriptors would produce
    /// the same policy template. This is required because Ledger policies use a
    /// single template with `/**` to represent both derivation paths.
    ///
    /// # Arguments
    ///
    /// * `value` - A `SubwalletDescriptorBackup` containing external and change descriptors
    ///
    /// # Returns
    ///
    /// A `LedgerPolicy` if both descriptors are compatible.
    ///
    /// # Errors
    ///
    /// Returns `Error::LedgerIncompatibleDescriptor` if:
    /// - The external and change descriptors would produce different policy templates
    /// - Either descriptor is not Ledger-compatible
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// # use btc_heritage_wallet::LedgerPolicy;
    /// # use btc_heritage::SubwalletDescriptorBackup;
    /// let backup = SubwalletDescriptorBackup {
    ///     external_descriptor: "tr([origin/86'/1'/0']xpub.../0/*)".parse().unwrap(),
    ///     change_descriptor: "tr([origin/86'/1'/0']xpub.../1/*)".parse().unwrap(),
    ///     ..
    /// };
    /// let policy = LedgerPolicy::try_from(backup).unwrap();
    /// // Results in: "tr(@0/**)"
    /// ```
    fn try_from(value: &SubwalletDescriptorBackup) -> Result<Self, Self::Error> {
        let external_descriptor = value.external_descriptor.to_string();
        let change_descriptor = value.change_descriptor.to_string();

        // Verify that both descriptors use the same account xpubs
        // This ensures they will produce the same policy template
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

    /// Test that validates a proper Heritage subwallet backup can be converted to a Ledger policy
    ///
    /// This test uses a valid backup with external and change descriptors that have:
    /// - The same account xpubs in both descriptors
    /// - Proper Heritage account xpub format with origin information
    /// - Correct Taproot structure with script paths
    /// - Time-based constraints (older() and after() conditions)
    #[test]
    fn from_valid_backup() {
        let valid_backup = r#"{
            "external_descriptor": "tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/0/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/0/*),and_v(v:older(12960),after(1731536000))))",
            "change_descriptor": "tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/1/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/1/*),and_v(v:older(12960),after(1731536000))))"
        }"#;
        let valid_backup: SubwalletDescriptorBackup = serde_json::from_str(valid_backup).unwrap();
        assert!(LedgerPolicy::try_from(&valid_backup).is_ok())
    }

    /// Test that validates rejection of incompatible subwallet backups
    ///
    /// This test uses an invalid backup where the descriptors contain keys that are not
    /// Heritage account xpubs. The keys in this example are raw public keys rather than
    /// extended public keys, which violates Ledger policy requirements.
    #[test]
    fn from_invalid_backup() {
        let invalid_backup = r#"{
        "external_descriptor": "tr([44990794/86'/1'/0']tpubDE9DbziEKzUWbomb29YUwersoSERpmogW115aoegGezrf2uKJZfTqNCD5it8u8AzAuDUoCBcGgmwKppcFSEJ4fuLvBTDLsm5hmeK6L7LZcz/0/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1706602192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1722154192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1737706192)))}})",
        "change_descriptor": "tr([44990794/86'/1'/1']tpubDE9DbziEKzUWdSo28yKWmuEcgaXEF6tP11EB39RiZN5DW5XCEXRhWbMVRBsPv7yuWHBuueuN7WAhQ3kbEdvg4uMfCvwEYd8ay344UtfsWtz/1/*,{and_v(v:pk([99ccb69a/86'/1'/1751476594'/0/0]02ee39732e7f49cf4c9bd9b3faec01ed6f62a668fef33fbec0f2708e4cebf5bc9b),and_v(v:older(8640),after(1706602192))),{and_v(v:pk([00bdc67c/86'/1'/1751476594'/0/0]03cb072f51f73029ba3023ee0ffb0caa0070ecde5fb849783579c6f8a9b9029157),and_v(v:older(17280),after(1722154192))),and_v(v:pk([53c80c75/86'/1'/1751476594'/0/0]035133a7acfda43784341da5e23a1ecd1ac25be2ded8ceaff151a9a4cd78199b20),and_v(v:older(25920),after(1737706192)))}})"
    }"#;
        let invalid_backup: SubwalletDescriptorBackup =
            serde_json::from_str(invalid_backup).unwrap();
        assert!(
            LedgerPolicy::try_from(&invalid_backup).is_err_and(|e| match e {
                Error::LedgerIncompatibleDescriptor(msg) =>
                    msg == "external and change descriptor templates would be different",
                _ => unreachable!("Only LedgerIncompatibleDescriptor errors can be raised"),
            })
        )
    }

    /// Test that validates a properly formatted Heritage Taproot descriptor
    ///
    /// This test ensures that descriptors with:
    /// - Proper Taproot structure (tr(...))
    /// - Valid Heritage account xpubs with origin information
    /// - Correct derivation paths using `/*` pattern
    /// - Script paths with time-based constraints
    /// are accepted and converted to Ledger policies.
    #[test]
    fn valid_descriptor_1() {
        let valid_descriptor = r"tr([9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/*,and_v(v:pk([f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*),and_v(v:older(12960),after(1731536000))))";
        assert!(LedgerPolicy::try_from(valid_descriptor).is_ok())
    }

    /// Test that validates rejection of non-Taproot descriptors
    ///
    /// This test ensures that descriptors using other script types (like wsh)
    /// are rejected, as Ledger policies only support Taproot descriptors.
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

    /// Test that validates rejection of descriptors with invalid key types
    ///
    /// This test ensures that descriptors containing raw public keys instead of
    /// Heritage account xpubs are rejected. The descriptor uses individual public
    /// keys with specific derivation paths instead of extendable account xpubs.
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
