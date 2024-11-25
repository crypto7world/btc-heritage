use crate::{async_client::auth::DeviceAuthorizationResponse, errors::Result};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(flatten)]
    pub(super) inner: crate::async_client::Tokens,
}
impl Default for Tokens {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

/// A trait providing methods for the OAuth tokens to be cached and retrieved
pub trait TokenCache {
    fn save_tokens(&mut self, tokens: &Tokens) -> Result<()>;
    fn load_tokens(&self) -> Result<Option<Tokens>>;
}

impl Tokens {
    pub fn new<F>(auth_url: &str, client_id: &str, callback: F) -> Result<Self>
    where
        F: FnOnce(DeviceAuthorizationResponse) -> Result<()>,
    {
        let blocker = super::blocker();
        let inner = blocker.block_on(crate::async_client::Tokens::new(
            auth_url,
            client_id,
            |dar| async { callback(dar) },
        ))?;
        Ok(Self { inner })
    }

    pub fn save<T: TokenCache>(&self, db: &mut T) -> Result<()> {
        db.save_tokens(&self)
    }

    pub fn load<T: TokenCache>(db: &T) -> Result<Option<Self>> {
        db.load_tokens()
    }
}
