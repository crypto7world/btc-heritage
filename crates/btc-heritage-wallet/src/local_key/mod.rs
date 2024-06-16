use bip39::Mnemonic;
use btc_heritage::bitcoin::{bip32::Fingerprint, Network};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalKey {
    mnemonic: Mnemonic,
    network: Network,
    fingerprint: Fingerprint,
    with_passphrase: bool,
}
