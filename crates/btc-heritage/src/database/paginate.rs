use core::ops::Deref;

use serde::{Deserialize, Serialize};

/// A paginated result containing a page of items and an optional continuation token
///
/// This struct wraps a page of results from a paginated query along with
/// a continuation token that can be used to fetch the next page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paginated<T> {
    /// The items in this page
    pub page: Vec<T>,
    /// Token to fetch the next page, None if this is the last page
    pub continuation_token: Option<ContinuationToken>,
}
impl<T> Paginated<T> {
    /// Returns true if this is the last page of results
    ///
    /// This is determined by the absence of a continuation token.
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

/// Token used to continue paginated queries from where the previous page ended
///
/// This opaque token contains the necessary information to resume a paginated
/// query at the correct position. The internal format should not be relied upon
/// by client code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuationToken(pub String);
