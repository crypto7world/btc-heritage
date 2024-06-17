use crate::errors::{Error, Result};
use std::path::PathBuf;

/// Prepare the database directory
/// Takes a [&str] and returns a [PathBuf] after ensuring it is valid and
/// has been created if needed
pub(super) fn prepare_data_dir(home_path: &str) -> Result<PathBuf> {
    let dir = {
        if !home_path.starts_with("~") {
            PathBuf::from(home_path)
        } else if home_path == "~" {
            dirs_next::home_dir()
                .ok_or_else(|| Error::Generic("home dir not found".to_string()))
                .unwrap()
        } else {
            let mut home = dirs_next::home_dir()
                .ok_or_else(|| Error::Generic("home dir not found".to_string()))
                .unwrap();
            home.push(home_path.strip_prefix("~/").unwrap());
            home
        }
    };

    log::debug!("{}", dir.as_path().display());

    if !dir.exists() {
        log::info!("Creating data directory {}", dir.as_path().display());
        std::fs::create_dir_all(&dir).map_err(|e| {
            Error::Generic(format!(
                "Cannot create {}: {}",
                dir.as_path().display(),
                e.to_string()
            ))
        })?;
    }

    Ok(dir)
}
