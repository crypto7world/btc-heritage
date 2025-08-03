use crate::database::errors::{DbError, Result};
use std::path::Path;

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
