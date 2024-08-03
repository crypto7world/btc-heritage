use core::ops::Deref;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    pub page: Vec<T>,
    pub continuation_token: Option<ContinuationToken>,
}
impl<T> Paginated<T> {
    pub fn is_last_page(&self) -> bool {
        self.continuation_token.is_none()
    }
}
impl<T> Deref for Paginated<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.page
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationToken(pub String);
