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
