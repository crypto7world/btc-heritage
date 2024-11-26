pub use super::auth::Tokens;
use crate::{
    errors::{Error, Result},
    types::{AccountXPubWithStatus, HeritageWalletMeta, NewTx},
    Heir, HeirContact, HeirCreate, HeirUpdate, Heritage, HeritageWalletMetaCreate, NewTxDrainTo,
    Synchronization, UnsignedPsbt,
};
use btc_heritage::{
    bitcoin::{psbt::Psbt, Txid},
    heritage_wallet::{HeritageUtxo, TransactionSummary, WalletAddress},
    BlockInclusionObjective, HeritageConfig, HeritageWalletBackup,
};

use reqwest::{Client, Method};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct HeritageServiceClient {
    client: Client,
    service_api_url: Arc<str>,
    tokens: Arc<Mutex<Option<Tokens>>>,
}

pub(super) async fn req_builder_to_body(req: reqwest::RequestBuilder) -> Result<String> {
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
    pub fn new(service_api_url: String, tokens: Option<Tokens>) -> Self {
        Self {
            client: Client::new(),
            service_api_url: service_api_url.into(),
            tokens: Arc::new(Mutex::new(tokens)),
        }
    }

    pub fn has_tokens(&self) -> bool {
        self.tokens.lock().expect("invalid mutex state").is_some()
    }

    /// The tokens are interiorly-mutable due to the need of (maybe) renewing
    /// them when calling an API so we can also allow changing the tokens with
    /// an immutable ref on self. It allows to continue using the HeritageServiceClient
    /// as a single allocated, cheaply clonable struct.
    pub fn set_tokens(&self, tokens: Option<Tokens>) {
        let mut mutex_guard = self.tokens.lock().expect("invalid mutex state");
        *mutex_guard = tokens;
    }

    async fn api_call<T: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<T>,
    ) -> Result<serde_json::Value> {
        let api_endpoint = format!("{}/{path}", self.service_api_url);
        log::debug!("Initiating {method} {api_endpoint}");
        let req = self.client.request(method, &api_endpoint);

        let req = {
            let mut mutex_guard = self.tokens.lock().expect("invalid mutex state");
            let tokens = mutex_guard.as_mut().ok_or(Error::Unauthenticated)?;
            tokens.refresh_if_needed().await?;
            req.bearer_auth(&tokens.id_token.0)
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
        serde_json::from_value(
            self.api_call(Method::POST, &path, Some(account_xpubs))
                .await?,
        )?;
        Ok(())
    }

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

    pub async fn post_wallet_create_address(&self, wallet_id: &str) -> Result<String> {
        let path = format!("wallets/{wallet_id}/create-address");
        let mut ret: HashMap<String, String> =
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
