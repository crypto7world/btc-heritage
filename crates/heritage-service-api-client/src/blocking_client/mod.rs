mod auth;
mod client;

pub use crate::errors::Error;
pub use auth::{TokenCache, Tokens};
pub use client::HeritageServiceClient;

use std::sync::OnceLock;
fn blocker() -> &'static Blocker {
    static BLOCKER: OnceLock<Blocker> = OnceLock::new();
    BLOCKER.get_or_init(|| {
        match tokio::runtime::Handle::try_current() {
            // There is a tokio runtime
            // No matter what, we can only work if we have been put inside
            // a task::spawn_blocking
            Ok(h) => {
                log::debug!("Instantiating Blocker using the current tokio::runtime::Handle");
                Blocker::ContextRuntimeHandle(h)
            }
            Err(_) => {
                log::debug!("Instantiating Blocker creating a new tokio::current_thread_runtime");
                Blocker::OwnedRuntime(
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("I just pray it cannot fail"),
                )
            }
        }
    })
}

#[derive(Debug)]
enum Blocker {
    OwnedRuntime(tokio::runtime::Runtime),
    ContextRuntimeHandle(tokio::runtime::Handle),
}

impl Blocker {
    fn block_on<F>(&self, future: F) -> F::Output
    where
        F: std::future::Future,
    {
        match self {
            Blocker::ContextRuntimeHandle(h) => h.block_on(future),
            Blocker::OwnedRuntime(rt) => rt.block_on(future),
        }
    }
}
