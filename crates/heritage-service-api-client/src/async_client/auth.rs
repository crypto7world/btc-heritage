use std::future::Future;

use btc_heritage::utils::timestamp_now;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::errors::{Error, Result};

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
    fn save_tokens(&mut self, tokens: &Tokens) -> Result<()>;
    fn load_tokens(&self) -> Result<Option<Tokens>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Tokens {
    pub(crate) id_token: String,
    pub(crate) access_token: String,
    refresh_token: String,
    expiration_ts: u64,
    token_endpoint: String,
    client_id: String,
}

impl Tokens {
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
            std::thread::sleep(core::time::Duration::from_secs(sleep_interval));

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
                            id_token: tokens.id_token,
                            access_token: tokens.access_token,
                            refresh_token: tokens.refresh_token.ok_or_else(|| {
                                Error::Generic("Missing refresh token".to_owned())
                            })?,
                            expiration_ts: timestamp_now() + tokens.expires_in as u64,
                            token_endpoint: auth_url.to_owned(),
                            client_id: client_id.to_owned(),
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
    pub(super) async fn refresh_if_needed(&mut self) -> Result<()> {
        log::debug!("Tokens::refresh_if_needed");
        if self.expiration_ts > timestamp_now() + 30 {
            return Ok(());
        };

        log::debug!("Initiating Token refresh flow");
        let req = Client::new().post(&self.token_endpoint).form(&[
            ("client_id", self.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", self.refresh_token.as_str()),
        ]);
        let body = super::client::req_builder_to_body(req).await?;
        let token_response = serde_json::from_str::<TokenResponse>(&body)?;

        self.id_token = token_response.id_token;
        self.access_token = token_response.access_token;
        if let Some(refresh_token) = token_response.refresh_token {
            self.refresh_token = refresh_token;
        }
        Ok(())
    }

    pub fn save<T: TokenCache>(&self, db: &mut T) -> Result<()> {
        db.save_tokens(self)
    }

    pub fn load<T: TokenCache>(db: &T) -> Result<Option<Self>> {
        db.load_tokens()
    }
}
