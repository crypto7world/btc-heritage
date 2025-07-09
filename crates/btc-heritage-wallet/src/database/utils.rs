use crate::database::errors::{DbError, Result};
use std::path::Path;

use super::Database;

/// Prepare the database directory
/// Takes a [Path] and ensure it has been created if needed
pub(super) fn prepare_data_dir(data_dir_path: &Path) -> Result<()> {
    log::debug!("{}", data_dir_path.display());

    if !data_dir_path.exists() {
        log::info!("Creating data directory {}", data_dir_path.display());
        std::fs::create_dir_all(&data_dir_path).map_err(|e| {
            DbError::Generic(format!(
                "Cannot create {}: {}",
                data_dir_path.display(),
                e.to_string()
            ))
        })?;
    }
    Ok(())
}

pub async fn blocking_db_operation<R: Send + 'static, F: FnOnce(Database) -> R + Send + 'static>(
    db: Database,
    f: F,
) -> R {
    tokio::task::spawn_blocking(move || f(db)).await.unwrap()
}
