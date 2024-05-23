use bitcoin::bip32::Fingerprint;

struct LocalProvider {}

pub enum WalletKeysProvider {
    Local,
    HardwareWallet,
}

pub struct WalletKeys {
    pub fingerprint: Fingerprint,
    wallet_keys_provider: WalletKeysProvider,
}

#[cfg(test)]
mod tests {
    use super::*;
}
