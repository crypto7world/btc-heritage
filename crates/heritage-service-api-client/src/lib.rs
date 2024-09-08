#[cfg(feature = "client")]
mod auth;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
mod errors;

#[cfg(feature = "client")]
pub use auth::{TokenCache, Tokens};
#[cfg(feature = "client")]
pub use client::HeritageServiceClient;
#[cfg(feature = "client")]
pub use errors::Error;

mod types;
pub use types::*;
