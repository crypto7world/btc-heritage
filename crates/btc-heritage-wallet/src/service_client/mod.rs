use reqwest::blocking::Client;
use std::cell::RefCell;
pub use tokens::Tokens;

use crate::errors::{Error, Result};

mod tokens;

pub struct HeritageServiceClient {
    client: Client,
    service_url: String,
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
    pub fn new(service_url: String, tokens: Option<Tokens>) -> Self {
        Self {
            client: Client::new(),
            service_url,
            tokens: RefCell::new(tokens),
        }
    }

    fn api_call_list(&self, path: &str) -> Result<()> {
        let mut tokens_borrow = self.tokens.borrow_mut();
        let tokens = tokens_borrow.as_mut().ok_or(Error::Unauthenticated)?;
        tokens.refresh_if_needed()?;

        let api_endpoint = format!("{}/{path}", self.service_url);
        log::debug!("Initiating GET {api_endpoint}");
        let req = self.client.get(&api_endpoint).bearer_auth(&tokens.id_token);
        let body = req_builder_to_body(req)?;
        println!("{body}");
        Ok(())
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
}
