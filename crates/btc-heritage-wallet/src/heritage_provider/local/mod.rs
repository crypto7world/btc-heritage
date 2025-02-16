use btc_heritage::{
    database::HeritageDatabase, heritage_config::HeritageExplorerTrait,
    heritage_wallet::CreatePsbtOptions, HeritageWalletBackup, PartiallySignedTransaction,
    SpendingConfig,
};

use heritage_service_api_client::{Fingerprint, HeritageUtxo, TransactionSummary};

use serde::{Deserialize, Serialize};

use crate::{
    errors::{Error, Result},
    online_wallet::LocalHeritageWallet,
    BoundFingerprint, Broadcaster, Database,
};

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalWallet {
    fingerprint: Fingerprint,
    local_heritage_wallet: LocalHeritageWallet,
}

impl LocalWallet {
    pub async fn create(
        fingerprint: Fingerprint,
        db: &Database,
        backup: HeritageWalletBackup,
    ) -> Result<Self> {
        Ok(Self {
            fingerprint,
            local_heritage_wallet: LocalHeritageWallet::create(db, Some(backup), 6).await?,
        })
    }

    pub fn local_heritage_wallet(&self) -> &LocalHeritageWallet {
        &self.local_heritage_wallet
    }
    pub fn local_heritage_wallet_mut(&mut self) -> &mut LocalHeritageWallet {
        &mut self.local_heritage_wallet
    }

    async fn heritage_utxos(&self) -> Result<Vec<HeritageUtxo>> {
        let wallet = self.local_heritage_wallet.heritage_wallet().await?;
        Ok(tokio::task::block_in_place(|| {
            wallet.database().list_utxos()
        })?)
    }
}

impl super::HeritageProvider for LocalWallet {
    async fn list_heritages(&self) -> Result<Vec<super::Heritage>> {
        //let utxos = self.heritage_utxos().await?;
        let utxos = {
            let wallet = self.local_heritage_wallet.heritage_wallet().await?;
            tokio::task::block_in_place(|| wallet.database().list_utxos())?
        };
        let mut result = vec![];
        for utxo in utxos.into_iter() {
            let mut heir_config_iter = utxo.heritage_config.iter_heir_configs();

            // The `heir_maturity` will be None if we cannot spend this UTXO, else it will contain our maturity
            let heir_maturity = loop {
                // As long as there are still HeirConfig to explore
                if let Some(hc) = heir_config_iter.next() {
                    // Verify if the HC match our fingerprint
                    if hc.fingerprint() == self.fingerprint {
                        // If yes, then the UTXO is spendable by us, we retrieve the estimated maturity
                        let heir_spending_timestamp = utxo
                            .estimate_heir_spending_timestamp(hc)
                            .expect("cannot return none as heir_config is present");
                        // And break out of the loop
                        break Some(heir_spending_timestamp);
                    }
                } else {
                    // We reached the end of the iterator without matching our fingerprint
                    // Therefor we cannot spend this utxo, we break out of the loop with None
                    break None;
                }
            };

            // If we are able to spend (maturity is some)
            // Then we can push a new Heritage in the results
            if let Some(maturity) = heir_maturity {
                let next_heir_maturity = heir_config_iter.next().map(|hc| {
                    utxo.estimate_heir_spending_timestamp(hc)
                        .expect("cannot return none as heir_config is present")
                });
                result.push(super::Heritage {
                    // For a local wallet, this is irrelevant, just put the fingerprint
                    heritage_id: self.fingerprint.to_string(),
                    value: utxo.amount,
                    maturity,
                    next_heir_maturity,
                });
            }
        }
        Ok(result)
    }

    /// Create a PSBT for an Heir
    ///
    /// # Important Note
    /// Current implementation as a catch. It may happen that the HeritageWallet reference the Heir with multiple, different HeirConfigs
    ///
    /// In that case, create_psbt will create a PSBT for only one of them with no guarantee of which one. Once this PSBT is spend and the wallet re-synchronized
    /// calling the function again will create a PSBT for another HeirConfig. This can be repeated until all Heritages HeirConfigs as been exhausted.
    ///
    /// # Todo
    /// At some point we could either:
    /// - create a "super" PSBT that encapsulate the individual PSBT of each HeirConfig. This would be legit as the key_provider
    ///   component should be able to sign all inputs no matter the type of HeirConfig as long as the fingerprints matches
    /// - make use of `heritage_id` to allow the user to choose which HeirConfig he wants to spend from, but that would not be
    ///   much better than the current situation.
    async fn create_psbt(
        &self,
        _heritage_id: &str,
        drain_to: btc_heritage::bitcoin::Address,
    ) -> Result<(PartiallySignedTransaction, TransactionSummary)> {
        // First retrieve the first HeirConfig that match our fingerprint and can spend now
        let utxos = self.heritage_utxos().await?;
        let heir_config = utxos
            .iter()
            .map(|utxo| {
                utxo.heritage_config.iter_heir_configs().filter(|&hc| {
                    utxo.heritage_config
                        .get_heritage_explorer(hc)
                        .unwrap()
                        .get_spend_conditions()
                        .can_spend_now()
                })
            })
            .flatten()
            .filter(|&hc| hc.fingerprint() == self.fingerprint)
            .cloned()
            .next()
            .ok_or(Error::Generic("Nothing to spend".to_owned()))?;

        let wallet = self.local_heritage_wallet.heritage_wallet().await?;
        // Then create a PSBT for each one
        Ok(tokio::task::block_in_place(|| {
            wallet.create_heir_psbt(
                heir_config,
                SpendingConfig::DrainTo(drain_to),
                CreatePsbtOptions::default(),
            )
        })?)
    }
}

impl Broadcaster for LocalWallet {
    async fn broadcast(
        &self,
        psbt: PartiallySignedTransaction,
    ) -> Result<heritage_service_api_client::Txid> {
        self.local_heritage_wallet.broadcast(psbt).await
    }
}
impl BoundFingerprint for LocalWallet {
    fn fingerprint(&self) -> Result<Fingerprint> {
        Ok(self.fingerprint)
    }
}
