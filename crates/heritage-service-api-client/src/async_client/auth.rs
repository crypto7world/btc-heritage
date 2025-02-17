use std::future::Future;

use btc_heritage::{bitcoin::base64, utils::timestamp_now};
use reqwest::Client;
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
struct TokenResponse {
    id_token: String,
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: u32,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
enum DeviceFlowError {
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
    refresh_token: Box<str>,
    expiration_ts: u64,
    token_endpoint: Box<str>,
    client_id: Box<str>,
}

impl Tokens {
    /// Creates new [Tokens] using the provided `auth_url` and `client_id`
    ///
    /// The `callback` closure will receive the initial [DeviceAuthorizationResponse] so it
    /// can be e.g. displayed to the user.
    pub async fn new<F, Fut>(auth_url: &str, client_id: &str, callback: F) -> Result<Self>
    where
        F: FnOnce(DeviceAuthorizationResponse) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        log::debug!("Tokens::new - auth_url={auth_url} client_id={client_id}");
        let client = Client::new();

        log::debug!("Initiating Device Authentication flow");
        let req: reqwest::RequestBuilder = client
            .post(auth_url)
            .form(&[("client_id", client_id), ("scope", "openid profile email")]);
        let body = super::client::req_builder_to_body(req).await?;

        let device_auth_response: DeviceAuthorizationResponse = serde_json::from_str(&body)?;
        let auth_expiration_ts = timestamp_now() + device_auth_response.expires_in as u64;
        let device_code = device_auth_response.device_code.clone();
        let sleep_interval = device_auth_response.interval as u64;

        callback(device_auth_response).await?;

        loop {
            if timestamp_now() >= auth_expiration_ts {
                return Err(Error::AuthenticationProcessExpired);
            }
            tokio::time::sleep(core::time::Duration::from_secs(sleep_interval)).await;

            log::debug!("Trying to retrieve tokens");
            let req = client.post(auth_url).form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_code),
                ("client_id", client_id),
            ]);

            match super::client::req_builder_to_body(req).await {
                Ok(body) => {
                    log::debug!("Got a 2XX response from the device token API");
                    if let Ok(tokens) = serde_json::from_str::<TokenResponse>(&body) {
                        log::debug!("Got tokens!");
                        return Ok(Self {
                            id_token: Token(tokens.id_token.into()),
                            access_token: Token(tokens.access_token.into()),
                            refresh_token: tokens
                                .refresh_token
                                .ok_or_else(|| Error::Generic("Missing refresh token".to_owned()))?
                                .into(),
                            expiration_ts: timestamp_now() + tokens.expires_in as u64,
                            token_endpoint: auth_url.into(),
                            client_id: client_id.into(),
                        });
                    } else {
                        log::error!("Invalid response from the device token API: {body}");
                        return Err(Error::Generic(format!(
                            "Invalid response from the device token API: {body}"
                        )));
                    }
                }
                Err(Error::ApiErrorResponse { code, message }) if code == 400 => {
                    log::debug!("Got a 400 response from the device token API: {message}");
                    match serde_json::from_str::<DeviceFlowError>(&message)? {
                        DeviceFlowError::AccessDenied => return Err(Error::AuthenticationDenied),
                        DeviceFlowError::ExpiredToken => {
                            return Err(Error::AuthenticationProcessExpired)
                        }
                        DeviceFlowError::AuthorizationPending => {
                            log::debug!("No tokens available yet. Retrying.")
                        }
                        DeviceFlowError::SlowDown => log::warn!(
                            "Got a slow_down response from the token endpoint, it should not happen."
                        ),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Refresh the Tokens.
    ///
    /// # Errors
    /// Return an error if the tokens refresh failed
    pub async fn refresh(&mut self) -> Result<()> {
        log::debug!("Tokens::refresh");

        log::debug!("Initiating Token refresh flow");
        let req = Client::new().post(self.token_endpoint.as_ref()).form(&[
            ("client_id", self.client_id.as_ref()),
            ("grant_type", "refresh_token"),
            ("refresh_token", self.refresh_token.as_ref()),
        ]);
        let body = super::client::req_builder_to_body(req).await?;
        let token_response = serde_json::from_str::<TokenResponse>(&body)?;

        self.id_token = Token(token_response.id_token.into());
        self.access_token = Token(token_response.access_token.into());
        if let Some(refresh_token) = token_response.refresh_token {
            self.refresh_token = refresh_token.into();
        }
        self.expiration_ts = timestamp_now() + token_response.expires_in as u64;

        Ok(())
    }

    pub fn need_refresh(&self) -> bool {
        log::debug!("Tokens::need_refresh");
        self.expiration_ts < timestamp_now() + 30
    }

    pub fn id_token(&self) -> &Token {
        &self.id_token
    }
    pub fn access_token(&self) -> &Token {
        &self.access_token
    }

    pub async fn save<T: TokenCache>(&self, db: &mut T) -> Result<()> {
        db.save_tokens(self).await
    }

    pub async fn load<T: TokenCache>(db: &T) -> Result<Option<Self>> {
        db.load_tokens().await
    }
}
