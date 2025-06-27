mod v0tov1;

use serde::{Deserialize, Serialize};

use super::{
    dbitem::{impl_db_single_item, DatabaseSingleItem},
    errors::DbError,
    Database,
};

/// Database schema version for migration management
#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SchemaVersion(u32);

impl_db_single_item!(SchemaVersion, "schema_version");

impl SchemaVersion {
    /// Get the current schema version that this application supports
    const fn current() -> Self {
        Self(1)
    }

    /// Get all schema versions from the given version up to (but not including) the target version
    fn versions_between(from: Self, to: Self) -> Vec<Self> {
        if from < to {
            (from.0..to.0).map(Self).collect()
        } else {
            Vec::new()
        }
    }
}

/// Trait for database migration plans
pub trait MigrationPlan: Send + Sync {
    /// Execute the migration plan to upgrade the database schema
    ///
    /// This method should take the database from version N-1 to version N
    /// and must be implemented in a way that either succeeds completely
    /// or fails without leaving the database in an inconsistent state.
    ///
    /// # Arguments
    ///
    /// * `db` - Mutable reference to the database to migrate
    ///
    /// # Errors
    ///
    /// Returns a [DbError] if the migration fails for any reason
    fn migrate(&self, db: &mut Database) -> Result<(), DbError>;
    fn expected_version(&self) -> SchemaVersion;

    fn control_version(&self, db: &Database) -> Result<(), DbError> {
        let db_version_key = SchemaVersion::item_key();
        let db_version = db._get_item(db_version_key)?.unwrap_or_default();
        if db_version == self.expected_version() {
            Ok(())
        } else {
            Err(DbError::IncorrectSchemaVersion {
                expected: self.expected_version(),
                real: db_version,
            })
        }
    }
}

/// Get the migration plans needed to upgrade from one schema version to another
///
/// # Arguments
///
/// * `from` - The current schema version in the database
/// * `to` - The target schema version to upgrade to
///
/// # Returns
///
/// A vector of migration plans that need to be executed in order
///
/// # Errors
///
/// Returns a [DbError] if the migration path is invalid or if migration
/// plans are not available for the requested version range
fn migration_plans(
    from: SchemaVersion,
    to: SchemaVersion,
) -> Result<Vec<Box<dyn MigrationPlan>>, DbError> {
    if from > to {
        return Err(DbError::SchemaVersionTooNew {
            database_version: from,
            application_version: to,
        });
    }

    if from == to {
        return Ok(Vec::new());
    }

    SchemaVersion::versions_between(from, to)
        .into_iter()
        .map(|version| match version {
            SchemaVersion(0) => Ok(Box::new(v0tov1::MigrationV0toV1) as Box<dyn MigrationPlan>),
            _ => Err(DbError::MigrationPlanNotFound(version)),
        })
        .collect()
}

/// Check and perform database schema migration if needed
///
/// This function checks the current schema version stored in the database
/// and applies any necessary migration plans to bring it up to the current
/// application schema version.
///
/// # Arguments
///
/// * `db` - Mutable reference to the database to check and potentially migrate
///
/// # Errors
///
/// Returns a [DbError] if:
/// - The database schema version is newer than what this application supports
/// - A required migration plan is not found
/// - Any migration plan fails to execute
pub async fn migrate_database_if_needed(db: &mut Database) -> Result<(), DbError> {
    let current_version = SchemaVersion::current();

    // Get the stored schema version, defaulting to SchemaVersion(0) if not found
    let stored_version = match SchemaVersion::load(db).await {
        Ok(stored) => stored,
        Err(DbError::KeyDoesNotExists(_)) => {
            // Database doesn't have a schema version yet, return default (V0)
            SchemaVersion::default()
        }
        Err(e) => return Err(e),
    };

    // Check if database version is compatible
    if stored_version > current_version {
        return Err(DbError::SchemaVersionTooNew {
            database_version: stored_version,
            application_version: current_version,
        });
    }

    // If versions match, no migration needed
    if stored_version == current_version {
        log::debug!(
            "Database schema is up to date at version {:?}",
            stored_version
        );
        return Ok(());
    }

    // Get migration plans
    let plans = migration_plans(stored_version, current_version)?;

    if plans.is_empty() {
        log::debug!("No migration plans needed");
        return Ok(());
    }

    log::info!(
        "Migrating database schema from {:?} to {:?} using {} migration plan(s)",
        stored_version,
        current_version,
        plans.len()
    );

    // Execute migration plans in sequence
    for (i, plan) in plans.iter().enumerate() {
        log::info!("Executing migration plan {} of {}", i + 1, plans.len());
        // Control version
        plan.control_version(db)?;
        // Execute the migration plan
        plan.migrate(db)?;
    }

    log::info!("Database schema migration completed successfully");
    Ok(())
}
