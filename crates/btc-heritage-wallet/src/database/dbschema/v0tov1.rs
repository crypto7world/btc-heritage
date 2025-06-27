use redb::ReadableTable;

use crate::{
    database::dbschema::SchemaVersion, errors::DbError, DatabaseItem, DatabaseSingleItem,
    HeirWallet, Wallet,
};
pub struct MigrationV0toV1;

/*
 * For this migration we need to:
 *  - Drop every previous synchronizations of the local wallets
 *  - Insert SchemaVersion(1) into the DB
 */
impl super::MigrationPlan for MigrationV0toV1 {
    fn migrate(&self, db: &mut crate::Database) -> Result<(), crate::errors::DbError> {
        log::debug!("Migrating from SchemaVersion(0) to SchemaVersion(1)");
        // List all LocalWallets tables
        let wallets = db._query::<Wallet>(Wallet::item_key_prefix())?;
        log::debug!("Found {} wallets", wallets.len());
        let heirwallets = db._query::<HeirWallet>(HeirWallet::item_key_prefix())?;
        log::debug!("Found {} heirwallets", heirwallets.len());
        let table_names = wallets
            .iter()
            .filter_map(|w| match w.online_wallet() {
                crate::AnyOnlineWallet::Local(local_heritage_wallet) => {
                    log::debug!("Wallet ({}) has a Local OnlineWallet", w.name());
                    Some(local_heritage_wallet.heritage_wallet_id.as_str())
                }
                _ => None,
            })
            .chain(
                heirwallets
                    .iter()
                    .filter_map(|hw| match hw.heritage_provider() {
                        crate::AnyHeritageProvider::LocalWallet(local_wallet) => {
                            log::debug!(
                                "HeirWallet ({}) has a LocalWallet HeritageProvider",
                                hw.name()
                            );
                            Some(
                                local_wallet
                                    .local_heritage_wallet()
                                    .heritage_wallet_id
                                    .as_str(),
                            )
                        }
                        _ => None,
                    }),
            )
            .inspect(|tn| log::debug!("Found local wallet table name: {tn}"))
            .collect::<Vec<_>>();

        log::debug!("{} tables to process", table_names.len());

        let txn = db.internal_db.begin_write()?;

        // Cleanup the keys
        fn delete_key(key: &str) -> bool {
            // Delete all:
            // - HeritageUtxo (contains #h#)
            // - TxSummary (contains #y#)
            // - WalletBalance (contains #b#)
            // - FeeRate (contains #f#)
            key.contains("#h#") || key.contains("#y#") || key.contains("#b#") || key.contains("#f#")
        }
        for table_name in table_names {
            log::debug!("Processing table: {table_name}");
            let mut table = txn.open_table(
                redb::TableDefinition::<&'static str, &'static [u8]>::new(table_name),
            )?;
            table.retain(|key, _| {
                if delete_key(key) {
                    log::debug!("Table: {table_name}, Remove key: {key}");
                    false
                } else {
                    // Retain
                    true
                }
            })?;
        }
        log::debug!("All tables were processed");

        log::debug!("Writing the Correct SchemaVersion");
        let mut table = txn.open_table(db.table_def())?;
        let db_version_key = SchemaVersion::item_key();

        // The version should be None
        if table.get(db_version_key)?.is_none() {
            table.insert(
                db_version_key,
                serde_json::to_vec(&SchemaVersion(1))
                    .map_err(|e| DbError::serde(db_version_key, e))?
                    .as_slice(),
            )?;
            log::info!(
                "Commiting Database upgrade: new version is {:?}",
                SchemaVersion(1)
            );
            drop(table);
            txn.commit()?;
        } else {
            log::error!(
                "Abording Database upgrade: the DatabaseVersion was not {:?}",
                SchemaVersion(0)
            );
            drop(table);
            txn.abort()?;
        }
        Ok(())
    }

    fn expected_version(&self) -> SchemaVersion {
        SchemaVersion(0)
    }
}
