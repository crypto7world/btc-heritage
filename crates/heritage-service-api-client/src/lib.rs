mod types;
pub use types::*;

#[cfg(any(feature = "async_client", feature = "blocking_client"))]
pub mod errors;

#[cfg(feature = "async_client")]
pub mod async_client;
#[cfg(feature = "async_client")]
pub mod auth {
    pub use crate::async_client::auth::DeviceAuthorizationResponse;
}

#[cfg(all(feature = "async_client", not(feature = "blocking_client")))]
pub use async_client::*;

#[cfg(feature = "blocking_client")]
pub mod blocking_client;
#[cfg(all(feature = "blocking_client"))]
pub use blocking_client::*;
