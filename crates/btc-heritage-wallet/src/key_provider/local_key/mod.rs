use crate::{
    errors::{Error, Result},
    BoundFingerprint,
};
use bip39::Mnemonic;
use btc_heritage::{
    bitcoin::{
        bip32::{
            ChildNumber, DerivationPath, ExtendedPrivKey, ExtendedPubKey, Fingerprint, KeySource,
        },
        key::{KeyPair, Secp256k1, TapTweak, XOnlyPublicKey},
        psbt::Prevouts,
        secp256k1,
        sighash::{SighashCache, TapSighashType},
        taproot::Signature,
        Network, PublicKey,
    },
    miniscript::{
        descriptor::{DescriptorXKey, SinglePub, SinglePubKey, Wildcard},
        DescriptorPublicKey, ToPublicKey,
    },
    AccountXPub, HeirConfig, SingleHeirPubkey,
};
use serde::{Deserialize, Serialize};

use super::{HeirConfigType, MnemonicBackup};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalKey {
    mnemonic: Mnemonic,
    network: Network,
    fingerprint: Fingerprint,
    with_password: bool,
    #[serde(default, skip)]
    cached_password: Option<String>,
}
impl LocalKey {
    /// Generate a new LocalKey with a random Mnemonic
    ///
    /// # Panics
    /// Panics if the word_count is not 12, 18 or 24
    pub fn generate(word_count: usize, password: Option<String>, network: Network) -> Self {
        let entropy = match word_count {
            12 => secp256k1::rand::random::<[u8; 16]>().to_vec(),
            18 => secp256k1::rand::random::<[u8; 24]>().to_vec(),
            24 => secp256k1::rand::random::<[u8; 32]>().to_vec(),
            _ => panic!("word_count should have been 12, 18 or 24 (got {word_count})"),
        };
        let mnemo = Mnemonic::from_entropy(&entropy).expect("correct entropy");
        Self::restore(mnemo, password, network)
    }

    pub fn restore(mnemo: Mnemonic, password: Option<String>, network: Network) -> Self {
        let fingerprint = LocalKey::_xprv(&mnemo, password.as_ref().map(|s| s.as_str()), network)
            .fingerprint(&Secp256k1::signing_only());
        Self {
            mnemonic: mnemo,
            network,
            fingerprint,
            with_password: password.is_some(),
            cached_password: password,
        }
    }

    pub fn init_local_key(&mut self, password: Option<String>) -> Result<()> {
        if self.with_password {
            self.cached_password
                .replace(password.ok_or(Error::LocalKeyMissingPassword)?);
        }

        if self.xprv().fingerprint(&Secp256k1::signing_only()) != self.fingerprint {
            return Err(Error::IncoherentLocalKeyFingerprint);
        }

        Ok(())
    }

    pub fn require_password(&self) -> bool {
        self.with_password
    }

    fn _xprv(mnemo: &Mnemonic, password: Option<&str>, network: Network) -> ExtendedPrivKey {
        ExtendedPrivKey::new_master(network, &mnemo.to_seed_normalized(password.unwrap_or("")))
            .expect("I really don't see how it could fail")
    }

    fn xprv(&self) -> ExtendedPrivKey {
        LocalKey::_xprv(
            &self.mnemonic,
            self.cached_password.as_ref().map(|s| s.as_str()),
            self.network,
        )
    }
}

impl LocalKey {
    fn base_derivation_path(&self) -> DerivationPath {
        let cointype_path_segment = match self.network {
            Network::Bitcoin => 0,
            _ => 1,
        };
        let base_derivation_path = vec![
            ChildNumber::from_hardened_idx(86).unwrap(),
            ChildNumber::from_hardened_idx(cointype_path_segment).unwrap(),
        ];
        DerivationPath::from(base_derivation_path)
    }

    fn derive_xpub(
        &self,
        master_xprv: Option<ExtendedPrivKey>,
        path: DerivationPath,
    ) -> DescriptorXKey<ExtendedPubKey> {
        let xprv = master_xprv.unwrap_or_else(|| self.xprv());
        // Just to be clear, this is the master private key
        // This assertion should never fail
        assert!(
            xprv.depth == 0
                && xprv.child_number == ChildNumber::from(0)
                && xprv.parent_fingerprint == Fingerprint::from([0u8; 4])
        );

        let secp = Secp256k1::new();

        // Derivation path must start as expected
        let base_derivation_path = self.base_derivation_path();
        assert!(base_derivation_path
            .into_iter()
            .zip(path.into_iter())
            .all(|(l, r)| l == r));

        let derived_xprv = &xprv
            .derive_priv(&secp, &path)
            .expect("I really don't see how it could fail");
        let origin: KeySource = (self.fingerprint, path);
        DescriptorXKey {
            origin: Some(origin),
            xkey: ExtendedPubKey::from_priv(&secp, derived_xprv),
            derivation_path: DerivationPath::default(),
            wildcard: Wildcard::Unhardened,
        }
    }
}

impl super::KeyProvider for LocalKey {
    fn sign_psbt(
        &self,
        psbt: &mut btc_heritage::PartiallySignedTransaction,
    ) -> crate::errors::Result<usize> {
        let xprv = self.xprv();
        // Just to be clear, this is the master private key
        // This assertion should never fail
        assert!(
            xprv.depth == 0
                && xprv.child_number == ChildNumber::from(0)
                && xprv.parent_fingerprint == Fingerprint::from([0u8; 4])
        );

        let secp = Secp256k1::new();

        let mut sig_cache = SighashCache::new(&psbt.unsigned_tx);
        let witness_utxos = psbt
            .inputs
            .iter()
            .enumerate()
            .map(|(i, input)| {
                if let Some(wit_utxo) = &input.witness_utxo {
                    Some(wit_utxo.clone())
                } else if let Some(in_tx) = &input.non_witness_utxo {
                    let vout = psbt.unsigned_tx.input[i].previous_output.vout;
                    Some(in_tx.output[vout as usize].clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let all_witness_utxos = witness_utxos
            .iter()
            .filter_map(|input| input.as_ref())
            .collect::<Vec<_>>();

        log::debug!("PSBT has {} input(s)", psbt.inputs.len());
        let mut signatures_count = 0usize;
        let mut signed_inputs_count = 0usize;
        for input_index in 0..psbt.inputs.len() {
            // We completly ignore the bip32_derivation property of the PSBT
            // and go straight for the tap_key_origins as we are not expecting
            // to handle anything else
            let input = &psbt.inputs[input_index];

            if input.tap_key_origins.len() == 0 {
                log::warn!("Input #{input_index} is not a Taproot input");
                continue;
            };

            let mut signing_keys = input
                .tap_key_origins
                .iter()
                .map(|(pk, (_, keysource))| (*pk, keysource))
                .filter_map(|(pk, keysource)| {
                    // Verify that the key source matches the current wallet
                    // Extract the fingerprint and derivation path of the input
                    let (input_key_fingerprint, input_key_derivationpath) = keysource;
                    // Verify that the fingerprint match the one of our wallet
                    if *input_key_fingerprint == self.fingerprint {
                        log::info!("Input #{input_index} key [{input_key_fingerprint}/{input_key_derivationpath}] is ours");
                        Some((pk, input_key_derivationpath.clone()))
                    } else {
                        log::debug!("Input #{input_index} key [{input_key_fingerprint}/{input_key_derivationpath}] is not ours");
                        None
                    }
                })
                .collect::<Vec<_>>();

            if signing_keys.len() == 0 {
                log::warn!("Input #{input_index} is not for our wallet");
                continue;
            };

            if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
                log::info!("Input #{input_index} is for our wallet but already signed");
                signed_inputs_count += 1;
                continue;
            }

            let internalkey = input.tap_internal_key.ok_or_else(|| {
                // Should not happen
                log::error!(
                    "Input #{input_index} is a malformed Taproot input (no tap_internal_key)"
                );
                Error::Generic("Malformed Taproot input".to_owned())
            })?;

            // Select the key that will be used to sign
            // If multiple keys are avaiable, use the internal key, else use the first one
            let (public_key, full_path) = if signing_keys.len() > 1 {
                log::warn!("Input #{input_index} can be signed by multiple keys of our wallet");
                let index = signing_keys
                    .iter()
                    .position(|(pk, _)| *pk == internalkey)
                    .unwrap_or(0);
                signing_keys.remove(index)
            } else {
                signing_keys.remove(0)
            };

            let derived_key = {
                xprv.derive_priv(&secp, &full_path)
                    .expect("I really don't see how it could fail")
            };

            let computed_pk = XOnlyPublicKey::from(secp256k1::PublicKey::from_secret_key(
                &secp,
                &derived_key.private_key,
            ));
            if public_key != computed_pk {
                return Err(Error::Generic(format!(
                    "Could not derive the correct public key at [{}/{full_path}]",
                    self.fingerprint
                )));
            }

            let is_internal_key = public_key == internalkey;

            log::info!(
                "Signing input #{input_index} with privatekey derived at [{}/{full_path}] (is_internal_key={is_internal_key})",
                self.fingerprint
            );

            let sighash_ty = input
                .sighash_type
                .map(|ty| ty.taproot_hash_ty())
                .unwrap_or(Ok(TapSighashType::Default))
                .map_err(|e| {
                    log::error!("Input #{input_index} is a malformed Taproot input ({e})");
                    Error::Generic(format!("Malformed Taproot input ({e})"))
                })?;
            log::debug!("Input #{input_index}: sighash_ty={sighash_ty}");
            let prevouts = match sighash_ty {
                TapSighashType::Default
                | TapSighashType::All
                | TapSighashType::None
                | TapSighashType::Single => {
                    if !witness_utxos.iter().all(Option::is_some) {
                        log::error!("Malformed PSBT: misses UTXO for some inputs");
                        return Err(Error::Generic(
                            "Malformed PSBT: misses UTXO for some inputs".to_owned(),
                        ));
                    }
                    Prevouts::All(&all_witness_utxos)
                }
                TapSighashType::AllPlusAnyoneCanPay
                | TapSighashType::NonePlusAnyoneCanPay
                | TapSighashType::SinglePlusAnyoneCanPay => Prevouts::One(
                    input_index,
                    witness_utxos[input_index].as_ref().ok_or_else(|| {
                        log::error!("Input #{input_index} misses an UTXO");
                        Error::Generic(format!("Malformed input #{input_index}: misses an UTXO"))
                    })?,
                ),
            };
            log::debug!("Input #{input_index}: prevouts={prevouts:?}");

            let leaf_hash_code_separator = if is_internal_key {
                None
            } else {
                // PSBT creation for heirs make it so there is infos for only one leaf for each Input
                // Therefor we sign only the leaf we have
                let Some((leaves, _)) = input.tap_key_origins.get(&public_key) else {
                    return Err(Error::Generic(
                        "Malformed PSBT: No TapLeaf hash for our signing key".to_owned(),
                    ));
                };
                if leaves.len() != 1 {
                    return Err(Error::Generic(
                        "Malformed PSBT: Multiple TapLeaf hash for our signing key".to_owned(),
                    ));
                }
                Some((leaves[0], 0xFFFFFFFF))
            };
            log::debug!(
                "Input #{input_index}: leaf_hash_code_separator={leaf_hash_code_separator:?}"
            );

            let sighash = sig_cache
                .taproot_signature_hash(
                    input_index,
                    &prevouts,
                    None,
                    leaf_hash_code_separator,
                    sighash_ty,
                )
                .map_err(|e| {
                    log::error!("Failled to computed sighash for Input #{input_index} ({e})");
                    Error::Generic(format!(
                        "Failled to computed sighash for Input #{input_index} ({e})"
                    ))
                })?;
            log::debug!("Input #{input_index}: sighash={sighash}");

            let keypair = KeyPair::from_seckey_slice(&secp, derived_key.private_key.as_ref())
                .expect("I really don't see how it could fail");
            let keypair = if is_internal_key {
                keypair.tap_tweak(&secp, input.tap_merkle_root).to_inner()
            } else {
                keypair // no tweak for script spend
            };

            let msg = &secp256k1::Message::from(sighash);
            let sig = secp.sign_schnorr(msg, &keypair);
            secp.verify_schnorr(&sig, msg, &keypair.public_key().to_x_only_pubkey())
                .expect("invalid or corrupted schnorr signature");

            let final_signature = Signature {
                sig,
                hash_ty: sighash_ty,
            };
            log::debug!("Input #{input_index}: final_signature={final_signature:?}");

            // Reborrow as mut
            let input = &mut psbt.inputs[input_index];
            if let Some((lh, _)) = leaf_hash_code_separator {
                input
                    .tap_script_sigs
                    .insert((public_key, lh), final_signature);
            } else {
                input.tap_key_sig = Some(final_signature);
            }

            signatures_count += 1;
            signed_inputs_count += 1;
        }
        log::info!(
            "{signed_inputs_count} signed/{} total input(s) ({signed_inputs_count} signed / {} already signed)",
            psbt.inputs.len(),
            signed_inputs_count - signatures_count
        );
        Ok(signatures_count)
    }

    fn derive_accounts_xpubs(
        &self,
        range: core::ops::Range<u32>,
    ) -> crate::errors::Result<Vec<AccountXPub>> {
        let xprv = self.xprv();
        let base_derivation_path = self.base_derivation_path();

        let xpubs = range
            .into_iter()
            .map(|i| {
                let derivation_path = base_derivation_path
                    .extend([ChildNumber::from_hardened_idx(i)
                        .map_err(|_| Error::AccountDerivationIndexOutOfBound(i))?]);
                let dxpub = self.derive_xpub(Some(xprv), derivation_path);
                let xpub = DescriptorPublicKey::XPub(dxpub);
                Ok(AccountXPub::try_from(xpub).expect("we ensured validity"))
            })
            .collect();
        xpubs
    }

    fn derive_heir_config(
        &self,
        heir_config_type: HeirConfigType,
    ) -> Result<btc_heritage::HeirConfig> {
        let base_derivation_path = self.base_derivation_path();
        let heir_derivation_path = base_derivation_path
            .extend([ChildNumber::from_hardened_idx(u32::from_be_bytes(*b"heir")).unwrap()]);
        let heir_xpub = self.derive_xpub(None, heir_derivation_path);

        match heir_config_type {
            HeirConfigType::SingleHeirPubkey => {
                let derivation_path = [
                    ChildNumber::from_normal_idx(0).unwrap(),
                    ChildNumber::from_normal_idx(0).unwrap(),
                ];
                let secp = Secp256k1::new();
                let key = heir_xpub
                    .xkey
                    .derive_pub(&secp, &derivation_path)
                    .expect("I really don't see how it could fail");
                let full_path = heir_xpub
                    .origin
                    .expect("origin is present")
                    .1
                    .extend(derivation_path);
                Ok(HeirConfig::SingleHeirPubkey(
                    SingleHeirPubkey::try_from(DescriptorPublicKey::Single(SinglePub {
                        origin: Some((self.fingerprint, full_path)),
                        key: SinglePubKey::FullKey(PublicKey::new(key.public_key)),
                    }))
                    .expect("we ensured validity"),
                ))
            }
            HeirConfigType::HeirXPubkey => Ok(HeirConfig::HeirXPubkey(
                AccountXPub::try_from(DescriptorPublicKey::XPub(heir_xpub))
                    .expect("we ensured validity"),
            )),
        }
    }

    fn backup_mnemonic(&self) -> Result<MnemonicBackup> {
        Ok(MnemonicBackup {
            mnemonic: self.mnemonic.clone(),
            fingerprint: self.fingerprint,
            with_password: self.with_password,
        })
    }
}
impl BoundFingerprint for LocalKey {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}

#[cfg(test)]
mod tests {

    use crate::KeyProvider;

    use super::*;
    use btc_heritage::PartiallySignedTransaction;
    use std::{fmt::Write, str::FromStr};

    const NETWORK: Network = Network::Regtest;

    /// In order:
    /// - Owner/Recipients
    /// - Owner/Drain
    /// - Backup/Drain - Present
    /// - Wife/Drain - Present
    /// - Backup/Drain - Future
    /// - Wife/Drain - Future
    /// - Brother/Drain - Future
    const UNSIGNED_PSBTS: [&str; 7] = [
        "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",
    ];
    pub enum TestPsbt {
        OwnerRecipients,
        OwnerDrain,
        BackupPresent,
        WifePresent,
        BackupFuture,
        WifeFuture,
        BrotherFuture,
    }
    fn get_test_unsigned_psbt(tp: TestPsbt) -> PartiallySignedTransaction {
        PartiallySignedTransaction::from_str(UNSIGNED_PSBTS[tp as usize]).unwrap()
    }

    const KEY_PROVIDERS: [[&str; 2]; 5] = [
        [
            "owner_wallet",
            "owner owner owner owner owner owner owner owner owner owner owner panther"
        ],
        [
            "backup_wallet",
            "save save save save save save save save save save save same"
        ],
        [
            "wife_wallet",
            "wife wife wife wife wife wife wife wife wife wife wife wide"
        ],
        [
            "brother_wallet",
            "brother brother brother brother brother brother brother brother brother brother brother bronze"
        ],
        [
            "random_wallet",
            ""
        ],
    ];
    enum TestKeyProvider {
        Owner = 0,
        Backup = 1,
        Wife = 2,
        Brother = 3,
        Random = 4,
    }
    fn get_test_key_provider(tw: TestKeyProvider) -> LocalKey {
        match tw {
            TestKeyProvider::Random => LocalKey::generate(12, None, NETWORK),
            _ => LocalKey::restore(
                Mnemonic::parse(KEY_PROVIDERS[tw as usize][1]).unwrap(),
                None,
                NETWORK,
            ),
        }
    }

    // Verify the wallet ability to sign their PSBT
    fn wallet_can_sign(tkp: TestKeyProvider, tp: TestPsbt) -> bool {
        let local_key = get_test_key_provider(tkp);
        let mut psbt = get_test_unsigned_psbt(tp);
        // If the wallet can sign, more than 0 inputs will be signed
        local_key.sign_psbt(&mut psbt).unwrap() > 0
    }
    fn wallet_cannot_sign(tkp: TestKeyProvider, tp: TestPsbt) -> bool {
        !wallet_can_sign(tkp, tp)
    }

    #[test]
    fn owner_wallet_signature() {
        assert!(wallet_can_sign(
            TestKeyProvider::Owner,
            TestPsbt::OwnerDrain
        ));
        assert!(wallet_can_sign(
            TestKeyProvider::Owner,
            TestPsbt::OwnerRecipients
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Owner,
            TestPsbt::BackupPresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Owner,
            TestPsbt::WifePresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Owner,
            TestPsbt::BackupFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Owner,
            TestPsbt::WifeFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Owner,
            TestPsbt::BrotherFuture
        ));
    }

    #[test]
    fn backup_wallet_signature() {
        assert!(wallet_cannot_sign(
            TestKeyProvider::Backup,
            TestPsbt::OwnerDrain
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Backup,
            TestPsbt::OwnerRecipients
        ));
        assert!(wallet_can_sign(
            TestKeyProvider::Backup,
            TestPsbt::BackupPresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Backup,
            TestPsbt::WifePresent
        ));
        assert!(wallet_can_sign(
            TestKeyProvider::Backup,
            TestPsbt::BackupFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Backup,
            TestPsbt::WifeFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Backup,
            TestPsbt::BrotherFuture
        ));
    }

    #[test]
    fn wife_wallet_signature() {
        assert!(wallet_cannot_sign(
            TestKeyProvider::Wife,
            TestPsbt::OwnerDrain
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Wife,
            TestPsbt::OwnerRecipients
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Wife,
            TestPsbt::BackupPresent
        ));
        assert!(wallet_can_sign(
            TestKeyProvider::Wife,
            TestPsbt::WifePresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Wife,
            TestPsbt::BackupFuture
        ));
        assert!(wallet_can_sign(TestKeyProvider::Wife, TestPsbt::WifeFuture));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Wife,
            TestPsbt::BrotherFuture
        ));
    }

    #[test]
    fn brother_wallet_signature() {
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::OwnerDrain
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::OwnerRecipients
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::BackupPresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::WifePresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::BackupFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Brother,
            TestPsbt::WifeFuture
        ));
        assert!(wallet_can_sign(
            TestKeyProvider::Brother,
            TestPsbt::BrotherFuture
        ));
    }

    #[test]
    fn random_wallet_signature() {
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::OwnerDrain
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::OwnerRecipients
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::BackupPresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::WifePresent
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::BackupFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::WifeFuture
        ));
        assert!(wallet_cannot_sign(
            TestKeyProvider::Random,
            TestPsbt::BrotherFuture
        ));
    }

    // Verify the xpub generation
    #[test]
    fn xpub_generation() {
        let local_key = get_test_key_provider(TestKeyProvider::Owner);
        let xpubs = local_key
            .derive_accounts_xpubs(0..20)
            .unwrap()
            .into_iter()
            .map(|axp| axp.to_string())
            .collect::<Vec<_>>();

        assert_eq!(xpubs, vec![
            "[9c7088e3/86'/1'/0']tpubDD2pKf3K2M2oukBVyGLVBKhqMV2MC5jQ3ABYNY17tFUgkq8Y2M65yBmeZHiz9gwrYfYkCZqipP9pL5NGwkSSsS2dijy7Nus1DLJLr6FQyWv/*",
            "[9c7088e3/86'/1'/1']tpubDD2pKf3K2M2oygc9tQX4ze9o9sMmn738oHEiRTwxAWJyW7HyPYjYQKMrxznXmgWncr416q1htkCszdHg3tbGseUUQXoxFZmjdAbwU8HY9QX/*",
            "[9c7088e3/86'/1'/2']tpubDD2pKf3K2M2p2MS1LdNxnNPKY61JgpGp9VTHf1k3e8coJk4ud2BhkrxYQifa8buLnrCyUbJke4US5cVobaZLr9qU554oMdwucWZpYZj5t13/*",
            "[9c7088e3/86'/1'/3']tpubDD2pKf3K2M2p32v62yjk7gHUzr8Nsu7oz2KE7rAyPpNRfdiaGcaFpAgBZMXACByAiw85jBJCuEsiKxumh9zrS6KUNK3BTXuKSTCFzEzfYAr/*",
            "[9c7088e3/86'/1'/4']tpubDD2pKf3K2M2p77GVTKs7PJfPtqzRLKSJ9DsbZeYDmFKAJEqsDmeiBbiM63Usg48UYxyT3ZZGjE66683KaG7vDRSzvWWDejhkWG8VeHrL65d/*",
            "[9c7088e3/86'/1'/5']tpubDD2pKf3K2M2p9CrcSUDT5kZqhTw8WEG2E93wZiWgjYFdAMuBSAf1SvQY1UnHk9J4xFgcoMNziJsMyzhCxkpi5f9ivgdxGVQTnNuaLMBFnX2/*",
            "[9c7088e3/86'/1'/6']tpubDD2pKf3K2M2pBBMnCozXtNKMLmUvZVaVSYrtVcSqajc9XzQeyymLsRkCpkL8QP3cp7LKcrpb9D97n39gZ539zZ4ambwSUBLYoupwpXptv3X/*",
            "[9c7088e3/86'/1'/7']tpubDD2pKf3K2M2pDvwTsF57CaFck5btMQCXA4DMqHhWddcWZT5Fou6poCdE5iokEsZDkyJGsKhsPSuJ6QkoDuygZndvoEDFsutPKVV2vLdeYvE/*",
            "[9c7088e3/86'/1'/8']tpubDD2pKf3K2M2pHfUPiY626U2uh4fcsw5oBo7co5UUCZp1TetbjRNBax6szSxm4MzbQyAKdiRcYKV2xLMXnLfDDCJRkS3NgWT9LZYV1xwhGyQ/*",
            "[9c7088e3/86'/1'/9']tpubDD2pKf3K2M2pJWX51vGsEghdjCVeyT7Hv1e794ehZ7uJKn2zieD7u1VsbH9J5CJQYZQ7L4hkfU1HqEYbYd8fmKRG3V3t7NHfFDfFn9667tj/*",
            "[9c7088e3/86'/1'/10']tpubDD2pKf3K2M2pMbAdHCchpGb5s7Nm4vUBQHReDubziTowFAxEAKV6swJVDCDYwQZaxYiN4fk83BmHEoZbTNLRiCwCqYCdYKDykPyMzpEZN6h/*",
            "[9c7088e3/86'/1'/11']tpubDD2pKf3K2M2pNtnBBdpkyXEGjvZna3XBGxVT3Zj4u2DreNz2EJJJjqN5f5sDDdgQ1FHPWSEinG9QNVZC8uU8RgDacAoVCjQF3ZXy6aduMQj/*",
            "[9c7088e3/86'/1'/12']tpubDD2pKf3K2M2pSygWGZCvFE6ro8EmAojbGbTTVjDZzKM45KDKgHn4naLCFSJZLSQP2gL1YaLAAmYVRQh8rxwPPCEaGFdtvfPWDeV6cge2ytx/*",
            "[9c7088e3/86'/1'/13']tpubDD2pKf3K2M2pV5nuQ4Kn3awpL2UAfv28amHat8Mb9eFHnyw2iwEuTC58q73cVqoZ9FEgQ9yJiC79y44Q2aqysTWPX6ZSsvLo9gLunPwogr7/*",
            "[9c7088e3/86'/1'/14']tpubDD2pKf3K2M2pYjXBctMmYakTC4Sxc1jzsGGAVXdPz4raMj2oVUHSSDCtRVPbNHQVrMry6Updt65T6pg6igo4c1HSTN1AwnAWgoQs5gtMUCW/*",
            "[9c7088e3/86'/1'/15']tpubDD2pKf3K2M2pZK6tVcDjvHy9BG26xVGPrdd8a7QwBJWyVqCYQjtqBG9mH7iSfyb4dzhNBE4qvTEPfnZg4sqJca7ZPRuL6rP5j7AMkGkqhyt/*",
            "[9c7088e3/86'/1'/16']tpubDD2pKf3K2M2pcpczWN95dG2eUpqKKk9aQFWWxqvjmQwkVQbt9MJhrcS6Eq7oLYb6uKY8p3PEwVvCBy9pe7eKCXjYPeGZe6iXSWhVFAGFe43/*",
            "[9c7088e3/86'/1'/17']tpubDD2pKf3K2M2pg8oiXb2pcdz9QYgexBa43U2Wt1EDX2w8SoY9p8p55SZsdsABUJLbHy6Hfi19nHsRrELJ6L6ZYA9VuYb6FAryhxonDwf3YFL/*",
            "[9c7088e3/86'/1'/18']tpubDD2pKf3K2M2pidehdGHhWgQxbwK26FxGgZi7viZGJSyugbZNJgvhb5H1F6GHx817x6wpJ5bKjfP7XmXHyetu6ZVTi7fLxkAASWjohjzwSiM/*",
            "[9c7088e3/86'/1'/19']tpubDD2pKf3K2M2pm4JswF6uHWJMa4Radk1DEB5uEk5eKH145HefKLMKN71uCYFVLHU14JDaDNFERTN4yXzESP7tPpkeXTZm38girQors7bVmhh/*"
        ])
    }

    fn heir_shp_generation(tw: TestKeyProvider) -> String {
        let local_key = get_test_key_provider(tw);
        let HeirConfig::SingleHeirPubkey(shp) = local_key
            .derive_heir_config(HeirConfigType::SingleHeirPubkey)
            .unwrap()
        else {
            unreachable!()
        };
        shp.to_string()
    }
    // Verify the heirs xpub generation
    #[test]
    fn heirs_shp_generation() {
        assert_eq!(heir_shp_generation(TestKeyProvider::Backup), "[f0d79bf6/86'/1'/1751476594'/0/0]025dfb71d525758f58a22106a743b5dbed8f1af1ebee044c80eb7c381e3d3e8b20");
        assert_eq!(heir_shp_generation(TestKeyProvider::Wife), "[c907dcb9/86'/1'/1751476594'/0/0]029d47adc090487692bc8c31729085be2ade1a80aa72962da9f1bb80d99d0cd7bf");
        assert_eq!(heir_shp_generation(TestKeyProvider::Brother), "[767e581a/86'/1'/1751476594'/0/0]03f49679ef0089dda208faa970d7491cca8334bbe2ca541f527a6d7adf06a53e9e");
    }

    fn heir_xpub_generation(tw: TestKeyProvider) -> String {
        let local_key = get_test_key_provider(tw);
        let HeirConfig::HeirXPubkey(xk) = local_key
            .derive_heir_config(HeirConfigType::HeirXPubkey)
            .unwrap()
        else {
            unreachable!()
        };
        xk.to_string()
    }
    // Verify the heirs xpub generation
    #[test]
    fn heirs_xpub_generation() {
        assert_eq!(heir_xpub_generation(TestKeyProvider::Backup), "[f0d79bf6/86'/1'/1751476594']tpubDDFibSiSkFTfnLc4cG5X2wwkLjatiWbxb3T6PNbaCuv9uQpeq4i2sRrk7EKFgd56TTTHXpKDrW4JEDfsueAfLYC9CTPAung761RWMcWE3aP/*");
        assert_eq!(heir_xpub_generation(TestKeyProvider::Wife), "[c907dcb9/86'/1'/1751476594']tpubDCH1wd7tX4HBrvELXe92EbfPeqV1Za6YxjDueUnFqThFSSijSJdkbhc2ReLeLhJfbfXTLPE5kHsB7mPFmbw87mQ6d3QbaRaKo2DPMDpRHH8/*");
        assert_eq!(heir_xpub_generation(TestKeyProvider::Brother), "[767e581a/86'/1'/1751476594']tpubDDkHPEg5JvFW1r1VqA7vo8kzuuBRywUv2DhVRepUUar3M4bHKGUJnmaHKqketdzhzenZWVWvLDmoFMtsGqh6xz9tPEG7SRkATQsbvoxuu8J/*");
    }

    fn hex_string_to_bytes(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..=i + 1], 16).unwrap())
            .collect::<Vec<_>>()
    }
    fn bytes_to_hex_string<B: AsRef<[u8]>>(bytes: B) -> String {
        let bytes = bytes.as_ref();
        let mut s = String::with_capacity(2 * bytes.len());
        for byte in bytes {
            write!(s, "{:02x}", byte).unwrap();
        }
        s
    }

    // Verify mnemonic BIP39 English test vectors
    #[test]
    fn mnemonic_test_vectors() {
        // From https://github.com/trezor/python-mnemonic/blob/master/vectors.json
        let test_vectors = [
                [
                    "00000000000000000000000000000000",
                    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
                    "c55257c360c07c72029aebc1b53c05ed0362ada38ead3e3e9efa3708e53495531f09a6987599d18264c1e1c92f2cf141630c7a3c4ab7c81b2f001698e7463b04",
                    "xprv9s21ZrQH143K3h3fDYiay8mocZ3afhfULfb5GX8kCBdno77K4HiA15Tg23wpbeF1pLfs1c5SPmYHrEpTuuRhxMwvKDwqdKiGJS9XFKzUsAF"
                ],
                [
                    "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                    "legal winner thank year wave sausage worth useful legal winner thank yellow",
                    "2e8905819b8723fe2c1d161860e5ee1830318dbf49a83bd451cfb8440c28bd6fa457fe1296106559a3c80937a1c1069be3a3a5bd381ee6260e8d9739fce1f607",
                    "xprv9s21ZrQH143K2gA81bYFHqU68xz1cX2APaSq5tt6MFSLeXnCKV1RVUJt9FWNTbrrryem4ZckN8k4Ls1H6nwdvDTvnV7zEXs2HgPezuVccsq"
                ],
                [
                    "80808080808080808080808080808080",
                    "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
                    "d71de856f81a8acc65e6fc851a38d4d7ec216fd0796d0a6827a3ad6ed5511a30fa280f12eb2e47ed2ac03b5c462a0358d18d69fe4f985ec81778c1b370b652a8",
                    "xprv9s21ZrQH143K2shfP28KM3nr5Ap1SXjz8gc2rAqqMEynmjt6o1qboCDpxckqXavCwdnYds6yBHZGKHv7ef2eTXy461PXUjBFQg6PrwY4Gzq"
                ],
                [
                    "ffffffffffffffffffffffffffffffff",
                    "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo wrong",
                    "ac27495480225222079d7be181583751e86f571027b0497b5b5d11218e0a8a13332572917f0f8e5a589620c6f15b11c61dee327651a14c34e18231052e48c069",
                    "xprv9s21ZrQH143K2V4oox4M8Zmhi2Fjx5XK4Lf7GKRvPSgydU3mjZuKGCTg7UPiBUD7ydVPvSLtg9hjp7MQTYsW67rZHAXeccqYqrsx8LcXnyd"
                ],
                [
                    "000000000000000000000000000000000000000000000000",
                    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon agent",
                    "035895f2f481b1b0f01fcf8c289c794660b289981a78f8106447707fdd9666ca06da5a9a565181599b79f53b844d8a71dd9f439c52a3d7b3e8a79c906ac845fa",
                    "xprv9s21ZrQH143K3mEDrypcZ2usWqFgzKB6jBBx9B6GfC7fu26X6hPRzVjzkqkPvDqp6g5eypdk6cyhGnBngbjeHTe4LsuLG1cCmKJka5SMkmU"
                ],
                [
                    "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                    "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal will",
                    "f2b94508732bcbacbcc020faefecfc89feafa6649a5491b8c952cede496c214a0c7b3c392d168748f2d4a612bada0753b52a1c7ac53c1e93abd5c6320b9e95dd",
                    "xprv9s21ZrQH143K3Lv9MZLj16np5GzLe7tDKQfVusBni7toqJGcnKRtHSxUwbKUyUWiwpK55g1DUSsw76TF1T93VT4gz4wt5RM23pkaQLnvBh7"
                ],
                [
                    "808080808080808080808080808080808080808080808080",
                    "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter always",
                    "107d7c02a5aa6f38c58083ff74f04c607c2d2c0ecc55501dadd72d025b751bc27fe913ffb796f841c49b1d33b610cf0e91d3aa239027f5e99fe4ce9e5088cd65",
                    "xprv9s21ZrQH143K3VPCbxbUtpkh9pRG371UCLDz3BjceqP1jz7XZsQ5EnNkYAEkfeZp62cDNj13ZTEVG1TEro9sZ9grfRmcYWLBhCocViKEJae"
                ],
                [
                    "ffffffffffffffffffffffffffffffffffffffffffffffff",
                    "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo when",
                    "0cd6e5d827bb62eb8fc1e262254223817fd068a74b5b449cc2f667c3f1f985a76379b43348d952e2265b4cd129090758b3e3c2c49103b5051aac2eaeb890a528",
                    "xprv9s21ZrQH143K36Ao5jHRVhFGDbLP6FCx8BEEmpru77ef3bmA928BxsqvVM27WnvvyfWywiFN8K6yToqMaGYfzS6Db1EHAXT5TuyCLBXUfdm"
                ],
                [
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art",
                    "bda85446c68413707090a52022edd26a1c9462295029f2e60cd7c4f2bbd3097170af7a4d73245cafa9c3cca8d561a7c3de6f5d4a10be8ed2a5e608d68f92fcc8",
                    "xprv9s21ZrQH143K32qBagUJAMU2LsHg3ka7jqMcV98Y7gVeVyNStwYS3U7yVVoDZ4btbRNf4h6ibWpY22iRmXq35qgLs79f312g2kj5539ebPM"
                ],
                [
                    "7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f",
                    "legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth useful legal winner thank year wave sausage worth title",
                    "bc09fca1804f7e69da93c2f2028eb238c227f2e9dda30cd63699232578480a4021b146ad717fbb7e451ce9eb835f43620bf5c514db0f8add49f5d121449d3e87",
                    "xprv9s21ZrQH143K3Y1sd2XVu9wtqxJRvybCfAetjUrMMco6r3v9qZTBeXiBZkS8JxWbcGJZyio8TrZtm6pkbzG8SYt1sxwNLh3Wx7to5pgiVFU"
                ],
                [
                    "8080808080808080808080808080808080808080808080808080808080808080",
                    "letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic avoid letter advice cage absurd amount doctor acoustic bless",
                    "c0c519bd0e91a2ed54357d9d1ebef6f5af218a153624cf4f2da911a0ed8f7a09e2ef61af0aca007096df430022f7a2b6fb91661a9589097069720d015e4e982f",
                    "xprv9s21ZrQH143K3CSnQNYC3MqAAqHwxeTLhDbhF43A4ss4ciWNmCY9zQGvAKUSqVUf2vPHBTSE1rB2pg4avopqSiLVzXEU8KziNnVPauTqLRo"
                ],
                [
                    "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                    "zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo zoo vote",
                    "dd48c104698c30cfe2b6142103248622fb7bb0ff692eebb00089b32d22484e1613912f0a5b694407be899ffd31ed3992c456cdf60f5d4564b8ba3f05a69890ad",
                    "xprv9s21ZrQH143K2WFF16X85T2QCpndrGwx6GueB72Zf3AHwHJaknRXNF37ZmDrtHrrLSHvbuRejXcnYxoZKvRquTPyp2JiNG3XcjQyzSEgqCB"
                ],
                [
                    "9e885d952ad362caeb4efe34a8e91bd2",
                    "ozone drill grab fiber curtain grace pudding thank cruise elder eight picnic",
                    "274ddc525802f7c828d8ef7ddbcdc5304e87ac3535913611fbbfa986d0c9e5476c91689f9c8a54fd55bd38606aa6a8595ad213d4c9c9f9aca3fb217069a41028",
                    "xprv9s21ZrQH143K2oZ9stBYpoaZ2ktHj7jLz7iMqpgg1En8kKFTXJHsjxry1JbKH19YrDTicVwKPehFKTbmaxgVEc5TpHdS1aYhB2s9aFJBeJH"
                ],
                [
                    "6610b25967cdcca9d59875f5cb50b0ea75433311869e930b",
                    "gravity machine north sort system female filter attitude volume fold club stay feature office ecology stable narrow fog",
                    "628c3827a8823298ee685db84f55caa34b5cc195a778e52d45f59bcf75aba68e4d7590e101dc414bc1bbd5737666fbbef35d1f1903953b66624f910feef245ac",
                    "xprv9s21ZrQH143K3uT8eQowUjsxrmsA9YUuQQK1RLqFufzybxD6DH6gPY7NjJ5G3EPHjsWDrs9iivSbmvjc9DQJbJGatfa9pv4MZ3wjr8qWPAK"
                ],
                [
                    "68a79eaca2324873eacc50cb9c6eca8cc68ea5d936f98787c60c7ebc74e6ce7c",
                    "hamster diagram private dutch cause delay private meat slide toddler razor book happy fancy gospel tennis maple dilemma loan word shrug inflict delay length",
                    "64c87cde7e12ecf6704ab95bb1408bef047c22db4cc7491c4271d170a1b213d20b385bc1588d9c7b38f1b39d415665b8a9030c9ec653d75e65f847d8fc1fc440",
                    "xprv9s21ZrQH143K2XTAhys3pMNcGn261Fi5Ta2Pw8PwaVPhg3D8DWkzWQwjTJfskj8ofb81i9NP2cUNKxwjueJHHMQAnxtivTA75uUFqPFeWzk"
                ],
                [
                    "c0ba5a8e914111210f2bd131f3d5e08d",
                    "scheme spot photo card baby mountain device kick cradle pact join borrow",
                    "ea725895aaae8d4c1cf682c1bfd2d358d52ed9f0f0591131b559e2724bb234fca05aa9c02c57407e04ee9dc3b454aa63fbff483a8b11de949624b9f1831a9612",
                    "xprv9s21ZrQH143K3FperxDp8vFsFycKCRcJGAFmcV7umQmcnMZaLtZRt13QJDsoS5F6oYT6BB4sS6zmTmyQAEkJKxJ7yByDNtRe5asP2jFGhT6"
                ],
                [
                    "6d9be1ee6ebd27a258115aad99b7317b9c8d28b6d76431c3",
                    "horn tenant knee talent sponsor spell gate clip pulse soap slush warm silver nephew swap uncle crack brave",
                    "fd579828af3da1d32544ce4db5c73d53fc8acc4ddb1e3b251a31179cdb71e853c56d2fcb11aed39898ce6c34b10b5382772db8796e52837b54468aeb312cfc3d",
                    "xprv9s21ZrQH143K3R1SfVZZLtVbXEB9ryVxmVtVMsMwmEyEvgXN6Q84LKkLRmf4ST6QrLeBm3jQsb9gx1uo23TS7vo3vAkZGZz71uuLCcywUkt"
                ],
                [
                    "9f6a2878b2520799a44ef18bc7df394e7061a224d2c33cd015b157d746869863",
                    "panda eyebrow bullet gorilla call smoke muffin taste mesh discover soft ostrich alcohol speed nation flash devote level hobby quick inner drive ghost inside",
                    "72be8e052fc4919d2adf28d5306b5474b0069df35b02303de8c1729c9538dbb6fc2d731d5f832193cd9fb6aeecbc469594a70e3dd50811b5067f3b88b28c3e8d",
                    "xprv9s21ZrQH143K2WNnKmssvZYM96VAr47iHUQUTUyUXH3sAGNjhJANddnhw3i3y3pBbRAVk5M5qUGFr4rHbEWwXgX4qrvrceifCYQJbbFDems"
                ],
                [
                    "23db8160a31d3e0dca3688ed941adbf3",
                    "cat swing flag economy stadium alone churn speed unique patch report train",
                    "deb5f45449e615feff5640f2e49f933ff51895de3b4381832b3139941c57b59205a42480c52175b6efcffaa58a2503887c1e8b363a707256bdd2b587b46541f5",
                    "xprv9s21ZrQH143K4G28omGMogEoYgDQuigBo8AFHAGDaJdqQ99QKMQ5J6fYTMfANTJy6xBmhvsNZ1CJzRZ64PWbnTFUn6CDV2FxoMDLXdk95DQ"
                ],
                [
                    "8197a4a47f0425faeaa69deebc05ca29c0a5b5cc76ceacc0",
                    "light rule cinnamon wrap drastic word pride squirrel upgrade then income fatal apart sustain crack supply proud access",
                    "4cbdff1ca2db800fd61cae72a57475fdc6bab03e441fd63f96dabd1f183ef5b782925f00105f318309a7e9c3ea6967c7801e46c8a58082674c860a37b93eda02",
                    "xprv9s21ZrQH143K3wtsvY8L2aZyxkiWULZH4vyQE5XkHTXkmx8gHo6RUEfH3Jyr6NwkJhvano7Xb2o6UqFKWHVo5scE31SGDCAUsgVhiUuUDyh"
                ],
                [
                    "066dca1a2bb7e8a1db2832148ce9933eea0f3ac9548d793112d9a95c9407efad",
                    "all hour make first leader extend hole alien behind guard gospel lava path output census museum junior mass reopen famous sing advance salt reform",
                    "26e975ec644423f4a4c4f4215ef09b4bd7ef924e85d1d17c4cf3f136c2863cf6df0a475045652c57eb5fb41513ca2a2d67722b77e954b4b3fc11f7590449191d",
                    "xprv9s21ZrQH143K3rEfqSM4QZRVmiMuSWY9wugscmaCjYja3SbUD3KPEB1a7QXJoajyR2T1SiXU7rFVRXMV9XdYVSZe7JoUXdP4SRHTxsT1nzm"
                ],
                [
                    "f30f8c1da665478f49b001d94c5fc452",
                    "vessel ladder alter error federal sibling chat ability sun glass valve picture",
                    "2aaa9242daafcee6aa9d7269f17d4efe271e1b9a529178d7dc139cd18747090bf9d60295d0ce74309a78852a9caadf0af48aae1c6253839624076224374bc63f",
                    "xprv9s21ZrQH143K2QWV9Wn8Vvs6jbqfF1YbTCdURQW9dLFKDovpKaKrqS3SEWsXCu6ZNky9PSAENg6c9AQYHcg4PjopRGGKmdD313ZHszymnps"
                ],
                [
                    "c10ec20dc3cd9f652c7fac2f1230f7a3c828389a14392f05",
                    "scissors invite lock maple supreme raw rapid void congress muscle digital elegant little brisk hair mango congress clump",
                    "7b4a10be9d98e6cba265566db7f136718e1398c71cb581e1b2f464cac1ceedf4f3e274dc270003c670ad8d02c4558b2f8e39edea2775c9e232c7cb798b069e88",
                    "xprv9s21ZrQH143K4aERa2bq7559eMCCEs2QmmqVjUuzfy5eAeDX4mqZffkYwpzGQRE2YEEeLVRoH4CSHxianrFaVnMN2RYaPUZJhJx8S5j6puX"
                ],
                [
                    "f585c11aec520db57dd353c69554b21a89b20fb0650966fa0a9d6f74fd989d8f",
                    "void come effort suffer camp survey warrior heavy shoot primary clutch crush open amazing screen patrol group space point ten exist slush involve unfold",
                    "01f5bced59dec48e362f2c45b5de68b9fd6c92c6634f44d6d40aab69056506f0e35524a518034ddc1192e1dacd32c1ed3eaa3c3b131c88ed8e7e54c49a5d0998",
                    "xprv9s21ZrQH143K39rnQJknpH1WEPFJrzmAqqasiDcVrNuk926oizzJDDQkdiTvNPr2FYDYzWgiMiC63YmfPAa2oPyNB23r2g7d1yiK6WpqaQS"
                ]
            ];
        let password = "TREZOR";
        for test_vector in test_vectors {
            let [v_entropy, v_mnemostr, v_key, v_xpriv] = test_vector;
            //let m = parse_mnemonic(v_mnemostr).unwrap();
            let mnemo = Mnemonic::from_entropy(&hex_string_to_bytes(v_entropy)).unwrap();
            let mnemostr = mnemo.to_string();
            let key = bytes_to_hex_string(mnemo.to_seed(password));
            let xpriv = LocalKey::restore(mnemo, Some(password.to_owned()), Network::Bitcoin)
                .xprv()
                .to_string();
            assert_eq!(mnemostr, v_mnemostr);
            assert_eq!(key, v_key);
            assert_eq!(xpriv, v_xpriv);
        }
    }
}
