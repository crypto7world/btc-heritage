use reqwest::{blocking::Client, Method};
use serde::Serialize;
use std::cell::RefCell;
pub use tokens::Tokens;

use crate::errors::{Error, Result};

mod tokens;
mod types;

pub struct HeritageServiceClient {
    client: Client,
    service_api_url: String,
    tokens: RefCell<Option<Tokens>>,
}

fn req_builder_to_body(req: reqwest::blocking::RequestBuilder) -> Result<String> {
    log::debug!("req={req:?}");
    let res = req.send()?;
    log::debug!("res={res:?}");
    let body_bytes = res.bytes()?.into();
    let body_str = String::from_utf8(body_bytes)?;
    log::debug!("body_str={body_str}");
    Ok(body_str)
}

impl HeritageServiceClient {
    pub fn new(service_api_url: String, tokens: Option<Tokens>) -> Self {
        Self {
            client: Client::new(),
            service_api_url,
            tokens: RefCell::new(tokens),
        }
    }

    fn api_call<T: Serialize>(&self, method: Method, path: &str, body: Option<T>) -> Result<()> {
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
        println!("{body}");
        Ok(())
    }

    fn api_call_list(&self, path: &str) -> Result<()> {
        self.api_call::<String>(Method::GET, path, None)
    }

    pub fn list_wallets(&self) -> Result<()> {
        self.api_call_list("wallets")
    }

    pub fn list_heirs(&self) -> Result<()> {
        self.api_call_list("heirs")
    }

    pub fn list_heritages(&self) -> Result<()> {
        self.api_call_list("heritages")
    }

    pub fn create_wallet(&self, name: &str) -> Result<()> {
        self.api_call_list("heritages")
    }
}
