use bip39::Mnemonic;
use btc_heritage::bitcoin::{bip32::Fingerprint, Network};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalKey {
    mnemonic: Mnemonic,
    network: Network,
    fingerprint: Fingerprint,
    with_password: bool,
}

impl super::WalletOffline for LocalKey {
    fn sign_psbt(
        &self,
        psbt: &mut btc_heritage::PartiallySignedTransaction,
    ) -> crate::errors::Result<usize> {
        todo!()
    }

    fn derive_accounts_xpubs(
        &self,
        count: usize,
    ) -> crate::errors::Result<Vec<btc_heritage::miniscript::DescriptorPublicKey>> {
        todo!()
    }

    fn derive_heir_xpub(
        &self,
    ) -> crate::errors::Result<btc_heritage::miniscript::DescriptorPublicKey> {
        todo!()
    }
}

impl crate::wallet::WalletCommons for LocalKey {
    fn fingerprint(&self) -> crate::errors::Result<Fingerprint> {
        todo!()
    }

    fn network(&self) -> crate::errors::Result<Network> {
        todo!()
    }
}
