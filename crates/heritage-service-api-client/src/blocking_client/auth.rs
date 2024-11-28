use crate::{async_client::auth::DeviceAuthorizationResponse, auth::Token, errors::Result};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(flatten)]
    pub(super) inner: crate::async_client::Tokens,
}

/// A trait providing methods for the OAuth tokens to be cached and retrieved
pub trait TokenCache {
    fn save_tokens(&mut self, tokens: &Tokens) -> Result<()>;
    fn load_tokens(&self) -> Result<Option<Tokens>>;
    fn clear(&mut self) -> Result<bool>;
}

impl Tokens {
    /// Creates new [Tokens] using the provided `auth_url` and `client_id`
    ///
    /// The `callback` closure will receive the initial [DeviceAuthorizationResponse] so it
    /// can be e.g. displayed to the user.
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

    /// Refresh the Tokens.
    ///
    /// # Errors
    /// Return an error if the tokens refresh failed
    pub fn refresh(&mut self) -> Result<()> {
        super::blocker().block_on(self.inner.refresh())
    }

    pub fn need_refresh(&self) -> bool {
        self.inner.need_refresh()
    }

    pub fn id_token(&self) -> &Token {
        self.inner.id_token()
    }
    pub fn access_token(&self) -> &Token {
        self.inner.access_token()
    }

    pub fn save<T: TokenCache>(&self, db: &mut T) -> Result<()> {
        db.save_tokens(&self)
    }

    pub fn load<T: TokenCache>(db: &T) -> Result<Option<Self>> {
        db.load_tokens()
    }
}
