pub use crate::auth::Tokens;
use crate::{
    errors::{Error, Result},
    types::{AccountXPubWithStatus, HeritageWalletMeta, NewTx},
    Synchronization,
};
use btc_heritage::{
    bitcoin::{psbt::Psbt, Txid},
    heritage_wallet::{HeritageUtxo, TransactionSummary, WalletAddress},
    BlockInclusionObjective, DescriptorsBackup, HeritageConfig,
};
use core::{cell::RefCell, str::FromStr};
use reqwest::{blocking::Client, Method};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug)]
pub struct HeritageServiceClient {
    client: Client,
    service_api_url: String,
    tokens: RefCell<Option<Tokens>>,
}

pub(crate) fn req_builder_to_body(req: reqwest::blocking::RequestBuilder) -> Result<String> {
    log::debug!("req={req:?}");
    let res = req.send()?;
    log::debug!("res={res:?}");
    let status_code = res.status();
    let body_bytes = res
        .bytes()
        .map_err(|e| {
            log::error!("Could not retrieve body bytes: {e}");
            Error::UnretrievableBodyResponse
        })?
        .into();
    let body_str = String::from_utf8(body_bytes).map_err(|e| {
        log::error!("Body is not valid UTF8: {e}");
        Error::UnretrievableBodyResponse
    })?;
    log::debug!("body_str={body_str}");
    if status_code.is_client_error() || status_code.is_server_error() {
        log::error!(
            "{} {}: {body_str}",
            status_code.as_u16(),
            status_code.canonical_reason().unwrap_or("UNKNOWN")
        );
        let mut error_body: HashMap<String, String> = serde_json::from_str(&body_str)?;
        let error_message = error_body.remove("message").unwrap_or(body_str);
        Err(Error::ApiErrorResponse {
            code: status_code.as_u16(),
            message: error_message,
        })
    } else {
        Ok(body_str)
    }
}

impl HeritageServiceClient {
    pub fn new(service_api_url: String, tokens: Option<Tokens>) -> Self {
        Self {
            client: Client::new(),
            service_api_url,
            tokens: RefCell::new(tokens),
        }
    }

    fn api_call<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<T>,
    ) -> Result<serde_json::Value> {
        let mut tokens_borrow = self.tokens.borrow_mut();
        let tokens = tokens_borrow.as_mut().ok_or(Error::Unauthenticated)?;
        tokens.refresh_if_needed()?;

        let api_endpoint = format!("{}/{path}", self.service_api_url);
        log::debug!("Initiating {method} {api_endpoint}");

        let req = self
            .client
            .request(method, &api_endpoint)
            .bearer_auth(&tokens.id_token);
        let req = match body {
            Some(body) => req.body(serde_json::to_string(&body)?),
            None => req,
        };
        let body = req_builder_to_body(req)?;
        match body.as_str() {
            "" => Ok(serde_json::Value::Null),
            _ => Ok(serde_json::from_str(&body)?),
        }
    }

    fn api_call_get(&self, path: &str) -> Result<serde_json::Value> {
        self.api_call::<String>(Method::GET, path, None)
    }

    ////////////////////////
    //      Wallets       //
    ////////////////////////

    pub fn list_wallets(&self) -> Result<Vec<HeritageWalletMeta>> {
        Ok(serde_json::from_value(self.api_call_get("wallets")?)?)
    }

    pub fn post_wallets(&self, name: &str) -> Result<crate::types::HeritageWalletMeta> {
        Ok(serde_json::from_value(self.api_call(
            Method::POST,
            "wallets",
            Some(json!({"name": name})),
        )?)?)
    }

    pub fn patch_wallet(
        &self,
        wallet_id: &str,
        name: Option<String>,
        block_inclusion_objective: Option<BlockInclusionObjective>,
    ) -> Result<HeritageWalletMeta> {
        let path = format!("wallets/{wallet_id}");
        Ok(serde_json::from_value(self.api_call(
            Method::PATCH,
            &path,
            Some(json!({"name": name, "block_inclusion_objective": block_inclusion_objective})),
        )?)?)
    }

    pub fn get_wallet(&self, wallet_id: &str) -> Result<HeritageWalletMeta> {
        let path = format!("wallets/{wallet_id}");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn list_wallet_account_xpubs(&self, wallet_id: &str) -> Result<Vec<AccountXPubWithStatus>> {
        let path = format!("wallets/{wallet_id}/account-xpubs");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn post_wallet_account_xpubs(
        &self,
        wallet_id: &str,
        account_xpubs: Vec<btc_heritage::AccountXPub>,
    ) -> Result<()> {
        let path = format!("wallets/{wallet_id}/account-xpubs");
        serde_json::from_value(self.api_call(Method::POST, &path, Some(account_xpubs))?)?;
        Ok(())
    }

    pub fn list_wallet_heritage_configs(&self, wallet_id: &str) -> Result<Vec<HeritageConfig>> {
        let path = format!("wallets/{wallet_id}/heritage-configs");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn post_wallet_heritage_configs(
        &self,
        wallet_id: &str,
        hc: HeritageConfig,
    ) -> Result<HeritageConfig> {
        let path = format!("wallets/{wallet_id}/heritage-configs");
        Ok(serde_json::from_value(self.api_call(
            Method::POST,
            &path,
            Some(hc),
        )?)?)
    }

    pub fn list_wallet_transactions(&self, wallet_id: &str) -> Result<Vec<TransactionSummary>> {
        let path = format!("wallets/{wallet_id}/tx-summaries");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn list_wallet_utxos(&self, wallet_id: &str) -> Result<Vec<HeritageUtxo>> {
        let path = format!("wallets/{wallet_id}/utxos");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn list_wallet_addresses(&self, wallet_id: &str) -> Result<Vec<WalletAddress>> {
        let path = format!("wallets/{wallet_id}/addresses");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn post_wallet_create_address(&self, wallet_id: &str) -> Result<String> {
        let path = format!("wallets/{wallet_id}/create-address");
        let mut ret: HashMap<String, String> =
            serde_json::from_value(self.api_call::<()>(Method::POST, &path, None)?)?;
        Ok(ret.remove("address").expect("trusting the api for now"))
    }

    pub fn post_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization> {
        let path = format!("wallets/{wallet_id}/synchronize");
        Ok(serde_json::from_value(self.api_call::<()>(
            Method::POST,
            &path,
            None,
        )?)?)
    }

    pub fn get_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization> {
        let path = format!("wallets/{wallet_id}/synchronize");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn get_wallet_descriptors_backup(&self, wallet_id: &str) -> Result<Vec<DescriptorsBackup>> {
        let path = format!("wallets/{wallet_id}/descriptors-backup");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn post_wallet_create_unsigned_tx(
        &self,
        wallet_id: &str,
        new_tx: NewTx,
    ) -> Result<(Psbt, TransactionSummary)> {
        let path = format!("wallets/{wallet_id}/create-unsigned-tx");
        let mut ret: HashMap<String, Value> =
            serde_json::from_value(self.api_call(Method::POST, &path, Some(new_tx))?)?;
        let psbt_str = ret.remove("psbt").expect("trusting the api for now");
        let tx_sum =
            serde_json::from_value(ret.remove("tx_summary").expect("trusting the api for now"))?;
        Ok((
            Psbt::from_str(psbt_str.as_str().expect("trusting the api for now"))
                .expect("trusting the api for now"),
            tx_sum,
        ))
    }

    pub fn post_broadcast_tx(&self, psbt: Psbt) -> Result<Txid> {
        let mut ret: HashMap<String, Txid> = serde_json::from_value(self.api_call(
            Method::POST,
            "broadcast-tx",
            Some(json!({"psbt": psbt.to_string()})),
        )?)?;
        Ok(ret.remove("txid").expect("trusting the api for now"))
    }

    ////////////////////////
    //       Heirs        //
    ////////////////////////
    pub fn list_heirs(&self) -> Result<()> {
        todo!()
    }

    pub fn post_heirs(&self) -> Result<()> {
        todo!()
    }

    pub fn patch_heir(&self) -> Result<()> {
        todo!()
    }

    pub fn post_heir_contacts(&self) -> Result<()> {
        todo!()
    }

    pub fn delete_heir_contacts(&self) -> Result<()> {
        todo!()
    }

    ////////////////////////
    //     Heritages      //
    ////////////////////////
    pub fn list_heritages(&self) -> Result<()> {
        todo!()
    }

    pub fn post_heritage_create_unsigned_tx(&self) -> Result<()> {
        todo!()
    }
}
