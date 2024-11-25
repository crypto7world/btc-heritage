use crate::{async_client::auth::DeviceAuthorizationResponse, errors::Result};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
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

    /// Refresh the Tokens if needed.
    ///
    /// Returns `true` if the token where refreshed, else return `false`.
    ///
    /// # Errors
    /// Return an error if the tokens needed to be refreshed but the process
    /// failed
    pub fn refresh_if_needed(&mut self) -> Result<bool> {
        super::blocker().block_on(self.inner.refresh_if_needed())
    }

    pub fn save<T: TokenCache>(&self, db: &mut T) -> Result<()> {
        db.save_tokens(&self)
    }

    pub fn load<T: TokenCache>(db: &T) -> Result<Option<Self>> {
        db.load_tokens()
    }
}
