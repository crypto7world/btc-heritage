use btc_heritage::PartiallySignedTransaction;
use heritage_service_api_client::{Fingerprint, Txid};

use crate::errors::Result;

pub use crate::heritage_provider::HeritageProvider;
pub use crate::key_provider::KeyProvider;
pub use crate::online_wallet::OnlineWallet;

/// For types that are bound to a specific [Fingerprint]
pub trait BoundFingerprint {
    /// Return the [Fingerprint] of the underlyng type
    fn fingerprint(&self) -> Result<Fingerprint>;
}

/// This trait provide the broadcasting capacity of fully signed PSBTs.
pub trait Broadcaster {
    /// Try to finalize and then broadcast the given [PartiallySignedTransaction],
    /// if successful returns the [Txid] of the new transaction.
    fn broadcast(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> impl std::future::Future<Output = Result<Txid>>;
}
