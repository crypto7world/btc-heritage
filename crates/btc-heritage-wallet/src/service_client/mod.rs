use btc_heritage::{AccountXPub, DescriptorsBackup};
use reqwest::{blocking::Client, Method};
use serde::Serialize;
use serde_json::json;
use std::cell::RefCell;
pub use tokens::Tokens;
pub use types::*;

use crate::errors::{Error, Result};

mod tokens;
mod types;

#[derive(Debug)]
pub struct HeritageServiceClient {
    client: Client,
    service_api_url: String,
    tokens: RefCell<Option<Tokens>>,
}

fn req_builder_to_body(req: reqwest::blocking::RequestBuilder) -> Result<String> {
    log::debug!("req={req:?}");
    let res = req.send()?;
    log::debug!("res={res:?}");
    let status_code = res.status();
    let body_bytes = res.bytes()?.into();
    let body_str = String::from_utf8(body_bytes)?;
    log::debug!("body_str={body_str}");
    if status_code.is_client_error() || status_code.is_server_error() {
        log::error!(
            "{} {}: {body_str}",
            status_code.as_u16(),
            status_code.canonical_reason().unwrap_or("UNKNOWN")
        );
        Err(Error::Generic(body_str))
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

    pub fn list_wallets(&self) -> Result<Vec<types::HeritageWalletMeta>> {
        Ok(serde_json::from_value(self.api_call_get("wallets")?)?)
    }

    pub fn list_heirs(&self) -> Result<serde_json::Value> {
        self.api_call_get("heirs")
    }

    pub fn list_heritages(&self) -> Result<serde_json::Value> {
        self.api_call_get("heritages")
    }

    pub fn create_wallet(&self, name: &str) -> Result<types::HeritageWalletMeta> {
        Ok(serde_json::from_value(self.api_call(
            Method::POST,
            "wallets",
            Some(json!({"name": name})),
        )?)?)
    }

    pub fn get_wallet(&self, wallet_id: &str) -> Result<types::HeritageWalletMeta> {
        let path = format!("wallets/{wallet_id}");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn get_descriptors_backup(&self, wallet_id: &str) -> Result<Vec<DescriptorsBackup>> {
        let path = format!("wallets/{wallet_id}/descriptors-backup");
        Ok(serde_json::from_value(self.api_call_get(&path)?)?)
    }

    pub fn post_wallet_account_xpubs(
        &self,
        wallet_id: &str,
        account_xpubs: &[btc_heritage::AccountXPub],
    ) -> Result<()> {
        let path = format!("wallets/{wallet_id}/account-xpubs");
        serde_json::from_value(self.api_call(Method::POST, &path, Some(account_xpubs))?)?;
        Ok(())
    }
}
