use btc_heritage::{bitcoin::base64, utils::timestamp_now};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::errors::{Error, Result};

#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Token(pub(crate) Box<str>);
impl Token {
    pub fn as_json(&self) -> Value {
        let start = self
            .0
            .find(".")
            .expect("correctly formed OAuth tokens always have 2 dots")
            + 1;
        let end = self.0[start..]
            .find(".")
            .expect("correctly formed OAuth tokens always have 2 dots")
            + start;
        let token_data =
            base64::decode(&self.0[start..end]).expect("between the 2 dots always valid B64");
        serde_json::from_slice(&token_data).expect("between the 2 dots always valid JSON")
    }
}

#[derive(Debug, Deserialize)]
pub struct DeviceAuthorizationResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval: u32,
    pub expires_in: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    id_token: String,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub(crate) enum DeviceFlowError {
    AccessDenied,
    ExpiredToken,
    AuthorizationPending,
    SlowDown,
}

/// A trait providing methods for the OAuth tokens to be cached and retrieved
pub trait TokenCache {
    fn save_tokens(
        &mut self,
        tokens: &Tokens,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
    fn load_tokens(&self) -> impl std::future::Future<Output = Result<Option<Tokens>>> + Send;
    fn clear(&mut self) -> impl std::future::Future<Output = Result<bool>> + Send;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Tokens {
    pub(crate) id_token: Token,
    pub(crate) access_token: Token,
    pub(crate) refresh_token: Box<str>,
    expiration_ts: u64,
}
impl Tokens {
    pub(crate) fn update_from_refresh_response(&mut self, token_resp: TokenResponse) {
        self.id_token = Token(token_resp.id_token.into());
        self.access_token = Token(token_resp.access_token.into());
        if let Some(refresh_token) = token_resp.refresh_token {
            self.refresh_token = refresh_token.into();
        }
        self.expiration_ts = timestamp_now() + token_resp.expires_in as u64;
    }
}
impl TryFrom<TokenResponse> for Tokens {
    type Error = Error;

    fn try_from(token_resp: TokenResponse) -> std::result::Result<Self, Self::Error> {
        Ok(Self {
            id_token: Token(token_resp.id_token.into()),
            access_token: Token(token_resp.access_token.into()),
            refresh_token: token_resp
                .refresh_token
                .ok_or_else(|| Error::Generic("Missing refresh token".to_owned()))?
                .into(),
            expiration_ts: timestamp_now() + token_resp.expires_in as u64,
        })
    }
}

impl Tokens {
    pub(crate) fn need_refresh(&self) -> bool {
        log::debug!("Tokens::need_refresh");
        self.expiration_ts < timestamp_now() + 30
    }

    pub fn id_token(&self) -> &Token {
        &self.id_token
    }
    pub fn access_token(&self) -> &Token {
        &self.access_token
    }

    pub async fn save<T: TokenCache>(&self, cache: &mut T) -> Result<()> {
        cache.save_tokens(self).await
    }

    pub async fn load<T: TokenCache>(cache: &T) -> Result<Option<Self>> {
        cache.load_tokens().await
    }
}
