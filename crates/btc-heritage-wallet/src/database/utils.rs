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

/// Executes a database operation in a blocking context using tokio's blocking thread pool
///
/// This function is used to perform database operations that may block the async runtime
/// by moving them to a dedicated blocking thread pool. This is essential for maintaining
/// async performance when dealing with potentially blocking database operations.
///
/// # Parameters
///
/// * `db` - The [Database] instance to pass to the operation
/// * `f` - A closure that takes a [Database] and returns a result of type `R`
///
/// # Returns
///
/// Returns the result of the closure execution
///
/// # Examples
///
/// ```no_run
/// use btc_heritage_wallet::{Database, db_utils::blocking_db_operation};
/// # async fn example(db: Database) -> Result<(), Box<dyn std::error::Error>> {
/// let result = blocking_db_operation(db, |db| {
///     // Perform blocking database operation here
///     db.get_item::<String>("some_key")
/// }).await;
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics if the spawned blocking task fails to execute (which should be rare
/// and indicates a serious system issue)
pub async fn blocking_db_operation<R: Send + 'static, F: FnOnce(Database) -> R + Send + 'static>(
    db: Database,
    f: F,
) -> R {
    tokio::task::spawn_blocking(move || f(db)).await.unwrap()
}
