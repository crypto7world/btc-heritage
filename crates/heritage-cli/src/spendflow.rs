use std::collections::HashMap;

use btc_heritage_wallet::{
    bitcoin::{psbt::Psbt, Network, Txid},
    btc_heritage::heritage_wallet::TransactionSummary,
    errors::Result,
    heritage_service_api_client::Fingerprint,
    Broadcaster, KeyProvider, PsbtSummary,
};

use crate::utils::ask_user_confirmation;

#[derive(Debug)]
pub struct SpendFlow<'a, KP: KeyProvider, B: Broadcaster> {
    psbt: Psbt,
    fingerprints: Option<&'a HashMap<Fingerprint, String>>,
    transaction_summary: Option<&'a TransactionSummary>,
    network: Network,
    skip_confirmation: bool,
    display_stage: bool,
    sign_stage: Option<&'a KP>,
    broadcast_stage: Option<&'a B>,
    summary_displayed: bool,
}

impl<'a, KP: KeyProvider, B: Broadcaster> SpendFlow<'a, KP, B> {
    pub fn new(psbt: Psbt, network: Network) -> Self {
        Self {
            psbt,
            fingerprints: None,
            transaction_summary: None,
            network,
            skip_confirmation: false,
            display_stage: false,
            sign_stage: None,
            broadcast_stage: None,
            summary_displayed: false,
        }
    }
    pub fn fingerprints(mut self, fingerprints: &'a HashMap<Fingerprint, String>) -> Self {
        self.fingerprints = Some(fingerprints);
        self
    }
    pub fn transaction_summary(mut self, transaction_summary: &'a TransactionSummary) -> Self {
        self.transaction_summary = Some(transaction_summary);
        self
    }
    pub fn _set_display(mut self, display: bool) -> Self {
        self.display_stage = display;
        self
    }
    pub fn display(mut self) -> Self {
        self.display_stage = true;
        self
    }
    pub fn set_sign(mut self, key_provider: Option<&'a KP>) -> Self {
        self.sign_stage = key_provider;
        self
    }
    pub fn sign(mut self, key_provider: &'a KP) -> Self {
        self.sign_stage = Some(key_provider);
        self
    }
    pub fn set_broadcast(mut self, broadcaster: Option<&'a B>) -> Self {
        self.broadcast_stage = broadcaster;
        self
    }
    pub fn broadcast(mut self, broadcaster: &'a B) -> Self {
        self.broadcast_stage = Some(broadcaster);
        self
    }
    pub fn set_skip_confirmations(mut self, skip_confirmation: bool) -> Self {
        self.skip_confirmation = skip_confirmation;
        self
    }
    pub fn _skip_confirmations(mut self) -> Self {
        self.skip_confirmation = true;
        self
    }
    pub fn run(mut self) -> Result<Box<dyn crate::display::Displayable>> {
        log::debug!("Starting SpendFlow::run");

        let display = self.display_stage;
        let sign = self.sign_stage.is_some();
        let broadcast = self.broadcast_stage.is_some();
        let skip_confirmations = self.skip_confirmation;

        let confirm_sign = display && sign && !skip_confirmations;
        let confirm_broadcast = sign && broadcast && !skip_confirmations;
        let display_summary_string = display || confirm_sign || confirm_broadcast;

        if display_summary_string {
            self.display_summary()?;
        };

        if confirm_sign && !self.confirm_before_sign()? {
            log::warn!("Signing refused");
            return Ok(Box::new(self.psbt.to_string()));
        };

        if sign {
            match self.run_sign() {
                Ok(_) => (),
                Err(e) => {
                    log::error!("Signing errored: {e}");
                    println!("Cannot sign the PSBT ({e})\n\n");
                    return Ok(Box::new(self.psbt.to_string()));
                }
            }
        };

        if display_summary_string {
            self.display_summary()?;
        };

        if confirm_broadcast && !self.confirm_before_broadcast()? {
            log::warn!("Broadcast refused");
            return Ok(Box::new(self.psbt.to_string()));
        };

        let result = if broadcast {
            let psbt = self.psbt.clone();
            let tx_id = match self.run_broadcast() {
                Ok(tx_id) => tx_id,
                Err(e) => {
                    log::error!("Broadcasting errored: {e}");
                    println!("Cannot broadcast the PSBT ({e})\n\n");
                    return Ok(Box::new(psbt.to_string()));
                }
            };
            tx_id.to_string()
        } else {
            self.psbt.to_string()
        };

        Ok(Box::new(result))
    }
    fn display_summary(&mut self) -> Result<()> {
        if !self.summary_displayed {
            log::debug!("{}", serde_json::to_string_pretty(&self.psbt)?);
            log::debug!("Creating the PSBT Summary string...");
            let summary = PsbtSummary::try_from((
                &self.psbt,
                self.transaction_summary,
                self.fingerprints,
                self.network,
            ))?;
            println!("################");
            println!("# PSBT Summary #");
            println!("################");
            println!("{summary}");
            self.summary_displayed = true;
        }
        Ok(())
    }
    fn confirm_before_sign(&self) -> Result<bool> {
        Ok(ask_user_confirmation("Do you want to sign this PSBT?")?)
    }
    fn run_sign(&mut self) -> Result<()> {
        log::debug!("{}", serde_json::to_string_pretty(&self.psbt)?);
        log::info!("Signing the PSBT...");
        // Sign it
        let signed_input_count = self
            .sign_stage
            .as_ref()
            .unwrap()
            .sign_psbt(&mut self.psbt)?;
        log::info!("Signed {signed_input_count} input(s)");
        log::debug!("{}", serde_json::to_string_pretty(&self.psbt)?);
        Ok(())
    }
    fn confirm_before_broadcast(&self) -> Result<bool> {
        Ok(ask_user_confirmation(
            "Do you want to broadcast this PSBT?",
        )?)
    }
    fn run_broadcast(self) -> Result<Txid> {
        log::debug!("{}", serde_json::to_string_pretty(&self.psbt)?);
        log::info!("Broadcasting the PSBT...");
        // Broadcast it
        let tx_id = self.broadcast_stage.unwrap().broadcast(self.psbt)?;
        log::info!("Transaction ID: {tx_id}");
        Ok(tx_id)
    }
}
