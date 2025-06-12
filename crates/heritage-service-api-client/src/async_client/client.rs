pub use super::auth::Tokens;
use super::{DeviceAuthorizationResponse, TokenCache};
use crate::{
    auth::{DeviceFlowError, TokenResponse},
    errors::{Error, Result},
    types::{AccountXPubWithStatus, HeritageWalletMeta, NewTx},
    Heir, HeirContact, HeirCreate, HeirUpdate, Heritage, HeritageWalletMetaCreate, NewTxDrainTo,
    SubwalletConfigMeta, Synchronization, UnsignedPsbt,
};
use btc_heritage::{
    bitcoin::{psbt::Psbt, Txid},
    heritage_wallet::{HeritageUtxo, TransactionSummary, WalletAddress},
    utils::timestamp_now,
    BlockInclusionObjective, HeritageConfig, HeritageWalletBackup,
};

use reqwest::{Client, Method};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap},
    future::Future,
    sync::Arc,
};
use tokio::sync::RwLock;

/// Heritage service configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeritageServiceConfig {
    /// Service API URL
    pub service_api_url: Arc<str>,
    /// Authentication URL
    pub auth_url: Arc<str>,
    /// Authentication client ID
    pub auth_client_id: Arc<str>,
}
impl Default for HeritageServiceConfig {
    fn default() -> Self {
        Self {
            service_api_url: Arc::from("https://api.btcherit.com/v1"),
            auth_url: Arc::from("https://device.crypto7.world/token"),
            auth_client_id: Arc::from("cda6031ca00d09d66c2b632448eb8fef"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HeritageServiceClient {
    client: Client,
    config: HeritageServiceConfig,
    tokens: Arc<RwLock<Option<Tokens>>>,
}
impl From<HeritageServiceConfig> for HeritageServiceClient {
    fn from(config: HeritageServiceConfig) -> Self {
        Self {
            client: Client::new(),
            config,
            tokens: Arc::new(RwLock::new(None)),
        }
    }
}

async fn req_builder_to_body(req: reqwest::RequestBuilder) -> Result<String> {
    log::debug!("req={req:?}");
    let res = req.send().await?;
    log::debug!("res={res:?}");
    let status_code = res.status();
    let body_bytes = res
        .bytes()
        .await
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
        log::debug!(
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
    pub async fn login<F, Fut>(&mut self, callback: F) -> Result<()>
    where
        F: FnOnce(DeviceAuthorizationResponse) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        log::debug!("HeritageServiceClient::login");

        log::debug!("Initiating Device Authentication flow");
        let req: reqwest::RequestBuilder = self.client.post(self.config.auth_url.as_ref()).form(&[
            ("client_id", self.config.auth_client_id.as_ref()),
            ("scope", "openid profile email"),
        ]);
        let body = req_builder_to_body(req).await?;

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
            let req = self.client.post(self.config.auth_url.as_ref()).form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", &device_code),
                ("client_id", self.config.auth_client_id.as_ref()),
            ]);

            match req_builder_to_body(req).await {
                Ok(body) => {
                    log::debug!("Got a 2XX response from the device token API");
                    if let Ok(token_resp) = serde_json::from_str::<TokenResponse>(&body) {
                        log::debug!("Got tokens!");
                        self.set_tokens(Some(Tokens::try_from(token_resp)?)).await;
                        return Ok(());
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

    pub async fn persist_tokens_in_cache<T: TokenCache>(&self, cache: &mut T) -> Result<()> {
        self.tokens
            .read()
            .await
            .as_ref()
            .ok_or(Error::Unauthenticated)?
            .save(cache)
            .await?;
        Ok(())
    }

    pub async fn load_tokens_from_cache<T: TokenCache>(&self, cache: &T) -> Result<()> {
        self.set_tokens(Tokens::load(cache).await?).await;
        Ok(())
    }

    pub async fn has_tokens(&self) -> bool {
        self.tokens.read().await.is_some()
    }

    pub fn get_tokens(&self) -> Arc<RwLock<Option<Tokens>>> {
        self.tokens.clone()
    }

    /// The tokens are interiorly-mutable due to the need of (maybe) renewing
    /// them when calling an API so we can also allow changing the tokens with
    /// an immutable ref on self. It allows to continue using the HeritageServiceClient
    /// as a single allocated, cheaply clonable struct.
    async fn set_tokens(&self, tokens: Option<Tokens>) {
        let mut mutex_guard = self.tokens.write().await;
        *mutex_guard = tokens;
    }

    /// Refresh the Tokens.
    ///
    /// # Errors
    /// Return an error if the tokens refresh failed
    async fn refresh_tokens(&self) -> Result<()> {
        log::debug!("HeritageServiceClient::refresh_tokens");

        if let Some(tokens) = self.tokens.write().await.as_mut() {
            if tokens.need_refresh() {
                log::debug!("Initiating Token refresh flow");
                let req = self.client.post(self.config.auth_url.as_ref()).form(&[
                    ("client_id", self.config.auth_client_id.as_ref()),
                    ("grant_type", "refresh_token"),
                    ("refresh_token", tokens.refresh_token.as_ref()),
                ]);
                let body = req_builder_to_body(req).await?;
                let token_resp = serde_json::from_str::<TokenResponse>(&body)?;

                tokens.update_from_refresh_response(token_resp);
            }
        }

        Ok(())
    }

    async fn api_call<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<T>,
    ) -> Result<serde_json::Value> {
        let api_endpoint = format!("{}/{path}", self.config.service_api_url);
        log::debug!("Initiating {method} {api_endpoint}");
        let req = self.client.request(method, &api_endpoint);

        let req = {
            let read_guard = self.tokens.read().await;
            let tokens = read_guard.as_ref().ok_or(Error::Unauthenticated)?;
            if tokens.need_refresh() {
                // Force drop the read guard
                drop(read_guard);
                self.refresh_tokens().await?;
                req.bearer_auth(&self.tokens.read().await.as_ref().unwrap().id_token.0)
            } else {
                req.bearer_auth(&tokens.id_token.0)
            }
        };

        let req = match body {
            Some(body) => {
                let body_str = serde_json::to_string(&body)?;
                log::debug!("body_str={body_str}");
                req.body(body_str)
            }
            None => req,
        };
        let body = req_builder_to_body(req).await?;
        match body.as_str() {
            "" => Ok(serde_json::Value::Null),
            _ => Ok(serde_json::from_str(&body)?),
        }
    }

    async fn api_call_get(&self, path: &str) -> Result<serde_json::Value> {
        self.api_call::<()>(Method::GET, path, None).await
    }

    ////////////////////////
    //      Wallets       //
    ////////////////////////
    pub async fn list_wallets(&self) -> Result<Vec<HeritageWalletMeta>> {
        Ok(serde_json::from_value(self.api_call_get("wallets").await?)?)
    }

    pub async fn post_wallets(
        &self,
        create: HeritageWalletMetaCreate,
    ) -> Result<crate::types::HeritageWalletMeta> {
        Ok(serde_json::from_value(
            self.api_call(Method::POST, "wallets", Some(create)).await?,
        )?)
    }

    pub async fn patch_wallet(
        &self,
        wallet_id: &str,
        name: Option<String>,
        block_inclusion_objective: Option<BlockInclusionObjective>,
    ) -> Result<HeritageWalletMeta> {
        let path = format!("wallets/{wallet_id}");
        let mut map = HashMap::new();
        if let Some(val) = name {
            map.insert("name", serde_json::to_value(val)?);
        }
        if let Some(val) = block_inclusion_objective {
            map.insert("block_inclusion_objective", serde_json::to_value(val)?);
        }
        Ok(serde_json::from_value(
            self.api_call(Method::PATCH, &path, Some(map)).await?,
        )?)
    }

    pub async fn get_wallet(&self, wallet_id: &str) -> Result<HeritageWalletMeta> {
        let path = format!("wallets/{wallet_id}");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn list_wallet_account_xpubs(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<AccountXPubWithStatus>> {
        let path = format!("wallets/{wallet_id}/account-xpubs");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn post_wallet_account_xpubs(
        &self,
        wallet_id: &str,
        account_xpubs: Vec<btc_heritage::AccountXPub>,
    ) -> Result<()> {
        let path = format!("wallets/{wallet_id}/account-xpubs");
        self.api_call(Method::POST, &path, Some(account_xpubs))
            .await?;
        Ok(())
    }

    pub async fn list_wallet_subwallet_configs(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<SubwalletConfigMeta>> {
        let path = format!("wallets/{wallet_id}/subwallet-configs");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    #[deprecated = "use list_wallet_subwallet_configs instead"]
    pub async fn list_wallet_heritage_configs(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<HeritageConfig>> {
        let path = format!("wallets/{wallet_id}/heritage-configs");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn post_wallet_heritage_configs(
        &self,
        wallet_id: &str,
        hc: HeritageConfig,
    ) -> Result<HeritageConfig> {
        let path = format!("wallets/{wallet_id}/heritage-configs");
        Ok(serde_json::from_value(
            self.api_call(Method::POST, &path, Some(hc)).await?,
        )?)
    }

    pub async fn list_wallet_transactions(
        &self,
        wallet_id: &str,
    ) -> Result<Vec<TransactionSummary>> {
        let path = format!("wallets/{wallet_id}/tx-summaries");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn list_wallet_utxos(&self, wallet_id: &str) -> Result<Vec<HeritageUtxo>> {
        let path = format!("wallets/{wallet_id}/utxos");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn list_wallet_addresses(&self, wallet_id: &str) -> Result<Vec<WalletAddress>> {
        let path = format!("wallets/{wallet_id}/addresses");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn post_wallet_create_address(&self, wallet_id: &str) -> Result<WalletAddress> {
        let path = format!("wallets/{wallet_id}/create-address");
        let mut ret: HashMap<String, WalletAddress> =
            serde_json::from_value(self.api_call::<()>(Method::POST, &path, None).await?)?;
        Ok(ret.remove("address").expect("trusting the api for now"))
    }

    pub async fn post_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization> {
        let path = format!("wallets/{wallet_id}/synchronize");
        Ok(serde_json::from_value(
            self.api_call::<()>(Method::POST, &path, None).await?,
        )?)
    }

    pub async fn get_wallet_synchronize(&self, wallet_id: &str) -> Result<Synchronization> {
        let path = format!("wallets/{wallet_id}/synchronize");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn get_wallet_descriptors_backup(
        &self,
        wallet_id: &str,
    ) -> Result<HeritageWalletBackup> {
        let path = format!("wallets/{wallet_id}/descriptors-backup");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn post_wallet_create_unsigned_tx(
        &self,
        wallet_id: &str,
        new_tx: NewTx,
    ) -> Result<(Psbt, TransactionSummary)> {
        let path = format!("wallets/{wallet_id}/create-unsigned-tx");
        let res: UnsignedPsbt =
            serde_json::from_value(self.api_call(Method::POST, &path, Some(new_tx)).await?)?;
        Ok(res.into())
    }

    pub async fn post_broadcast_tx(&self, psbt: Psbt) -> Result<Txid> {
        let mut ret: HashMap<String, Txid> = serde_json::from_value(
            self.api_call(
                Method::POST,
                "broadcast-tx",
                Some(json!({"psbt": psbt.to_string()})),
            )
            .await?,
        )?;
        Ok(ret.remove("txid").expect("trusting the api for now"))
    }

    ////////////////////////
    //       Heirs        //
    ////////////////////////
    pub async fn list_heirs(&self) -> Result<Vec<Heir>> {
        Ok(serde_json::from_value(self.api_call_get("heirs").await?)?)
    }

    pub async fn post_heirs(&self, heir_create: HeirCreate) -> Result<Heir> {
        Ok(serde_json::from_value(
            self.api_call(Method::POST, "heirs", Some(heir_create))
                .await?,
        )?)
    }

    pub async fn get_heir(&self, heir_id: &str) -> Result<Heir> {
        let path = format!("heirs/{heir_id}");
        Ok(serde_json::from_value(self.api_call_get(&path).await?)?)
    }

    pub async fn patch_heir(&self, heir_id: &str, heir_update: HeirUpdate) -> Result<Heir> {
        let path = format!("heirs/{heir_id}");
        Ok(serde_json::from_value(
            self.api_call(Method::PATCH, &path, Some(heir_update))
                .await?,
        )?)
    }
    #[cfg(feature = "client")]
    pub async fn post_heir_contacts(
        &self,
        heir_id: &str,
        contacts_to_add: Vec<HeirContact>,
    ) -> Result<()> {
        let path = format!("heirs/{heir_id}/contacts");
        let contacts_to_add: BTreeSet<HeirContact> = contacts_to_add.into_iter().collect();
        self.api_call(Method::POST, &path, Some(contacts_to_add))
            .await?;
        Ok(())
    }

    pub async fn delete_heir_contacts(
        &self,
        heir_id: &str,
        contacts_to_delete: Vec<HeirContact>,
    ) -> Result<()> {
        let path = format!("heirs/{heir_id}/contacts");
        let contacts_to_delete: BTreeSet<HeirContact> = contacts_to_delete.into_iter().collect();
        self.api_call(Method::DELETE, &path, Some(contacts_to_delete))
            .await?;
        Ok(())
    }

    ////////////////////////
    //     Heritages      //
    ////////////////////////
    pub async fn list_heritages(&self) -> Result<Vec<Heritage>> {
        Ok(serde_json::from_value(
            self.api_call_get("heritages").await?,
        )?)
    }

    pub async fn post_heritage_create_unsigned_tx(
        &self,
        heritage_id: &str,
        drain_to: NewTxDrainTo,
    ) -> Result<(Psbt, TransactionSummary)> {
        let path = format!("heritages/{heritage_id}/create-unsigned-tx");
        let res: UnsignedPsbt =
            serde_json::from_value(self.api_call(Method::POST, &path, Some(drain_to)).await?)?;
        Ok(res.into())
    }
}
