pub mod memory;
pub mod paginate;

use bdk::{database::BatchDatabase, BlockTime};
use core::fmt::Display;

use crate::{
    account_xpub::AccountXPub,
    bitcoin::{FeeRate, OutPoint, Txid},
    errors::DatabaseError,
    heritage_wallet::{
        BlockInclusionObjective, HeritageUtxo, HeritageWalletBalance, SubwalletConfigId,
        TransactionSummary,
    },
    subwallet_config::SubwalletConfig,
};

use self::paginate::{ContinuationToken, Paginated};

type Result<T> = core::result::Result<T, DatabaseError>;

/// Identifier for a database partition or subdatabase
///
/// The Database of a HeritageWallet is partitionned so each partition/subdatabase can serve
/// as a database for a BDK Wallet.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SubdatabaseId(String);
impl SubdatabaseId {
    /// Creates a new SubdatabaseId from any type that can be converted to a string
    pub fn from<T: ToString>(value: T) -> Self {
        Self(value.to_string())
    }
}
impl Display for SubdatabaseId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(f)
    }
}

/// Trait for databases that can be partitioned into subdatabases
///
/// This allows a single HeritageWallet database instance to store data for the multiple
/// BDK wallets it contains (one for each Heritage Configuration).
pub trait PartitionableDatabase {
    /// The type of subdatabase returned by this partitionable database
    type SubDatabase: BatchDatabase;
    /// Retrieves or creates a subdatabase with the given identifier
    ///
    /// # Errors
    ///
    /// Returns a database error if the subdatabase cannot be created or accessed.
    fn get_subdatabase(&self, subdatabase_id: SubdatabaseId) -> Result<Self::SubDatabase>;
}

/// Operations that can be run in a single transaction to ensure data consistency
///
/// This trait groups operations that must be executed atomically to maintain
/// database integrity, particularly when updating wallet configurations and
/// managing account extended public keys.
pub trait TransacHeritageOperation {
    /// Stores a new SubwalletConfig at the specified index
    ///
    /// The function **MUST** ensure that the new value does not override
    /// a previously stored value to prevent accidental data loss.
    ///
    /// # Errors
    ///
    /// Returns an error if a SubwalletConfig already exists at the given index
    /// or if the database operation fails.
    fn put_subwallet_config(
        &mut self,
        index: SubwalletConfigId,
        subwallet_config: &SubwalletConfig,
    ) -> Result<()>;

    /// Safely and atomically updates the current SubwalletConfig
    ///
    /// This function ensures that `new_subwallet_config` is stored only if
    /// `old_subwallet_config` matches the currently stored value, providing
    /// optimistic concurrency control.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::UnexpectedCurrentSubwalletConfig`] if the stored
    /// value doesn't match `old_subwallet_config`, or other database errors.
    fn safe_update_current_subwallet_config(
        &mut self,
        new_subwallet_config: &SubwalletConfig,
        old_subwallet_config: Option<&SubwalletConfig>,
    ) -> Result<()>;

    /// Deletes an unused AccountXPub after verifying it exists
    ///
    /// This function first verifies the AccountXPub is present in the database
    /// before deletion to detect potential concurrent operations.
    ///
    /// # Errors
    ///
    /// Returns [`DatabaseError::AccountXPubInexistant`] if the AccountXPub
    /// is not found in the database, which may indicate a concurrent operation.
    fn delete_unused_account_xpub(&mut self, account_xpub: &AccountXPub) -> Result<()>;
}

/// Main database trait for heritage wallet operations
///
/// This trait combines partitioning capabilities with transactional operations
/// and provides all the necessary methods for managing heritage wallet data,
/// including subwallet configurations, account keys, UTXOs, and transaction summaries.
pub trait HeritageDatabase: PartitionableDatabase + TransacHeritageOperation {
    /// Retrieves a SubwalletConfig by its index
    ///
    /// # Returns
    ///
    /// `Some(SubwalletConfig)` if found, `None` if no config exists at the given index.
    fn get_subwallet_config(&self, index: SubwalletConfigId) -> Result<Option<SubwalletConfig>>;

    /// Lists all SubwalletConfigs that are no longer current but still stored
    ///
    /// These are historical configurations that may still be needed for
    /// transaction history or recovery purposes.
    fn list_obsolete_subwallet_configs(&self) -> Result<Vec<SubwalletConfig>>;

    /// Returns an unused AccountXPub, if any available
    ///
    /// Should return the first available AccountXPub in AccountXPubId order
    /// to ensure deterministic behavior.
    ///
    /// # Returns
    ///
    /// `Some(AccountXPub)` if an unused key is available, `None` otherwise.
    fn get_unused_account_xpub(&self) -> Result<Option<AccountXPub>>;

    /// Returns all unused [AccountXPub]s assigned to the HeritageWallet
    ///
    /// These are AccountXPubs that have been added to the wallet but not yet
    /// used to create any SubwalletConfig.
    fn list_unused_account_xpubs(&self) -> Result<Vec<AccountXPub>>;

    /// Returns all [AccountXPub]s that have already been used
    ///
    /// These AccountXPubs have been used to create [SubwalletConfig]s and
    /// should not be reused to avoid address reuse and potential fund loss.
    fn list_used_account_xpubs(&self) -> Result<Vec<AccountXPub>>;

    /// Add new, available [AccountXPub] to the [HeritageWallet](crate::HeritageWallet).
    ///
    /// # Important
    /// You can **NEVER** reuse the same [AccountXPub::descriptor_id()] as an [AccountXPub]
    /// that have already been used and you **MUST** use [AccountXPub::descriptor_id()] as a
    /// unicity discriminent or it will break the [HeritageWallet](crate::HeritageWallet)
    /// functional design and could lead to a loss of coins.
    ///
    /// The reason for that is that the [AccountXPub::descriptor_id()] is used as the [SubwalletConfig::subwallet_id]
    /// for new [SubwalletConfig] of an [HeritageWallet](crate::HeritageWallet). Reusing an ID means
    /// overwritting an old [SubwalletConfig] and forgetting its descriptors and addresses: you would not
    /// be able to spend the coins anymore.
    ///
    /// Test your implementation of the trait against the test module provided with this lib
    /// and publicly usable with the feature `database-tests`
    fn add_unused_account_xpubs(&mut self, account_xpubs: &Vec<AccountXPub>) -> Result<()>;

    /// Adds new [HeritageUtxo]s to the database
    ///
    /// Existing UTXOs with the same [OutPoint] will be overridden.
    fn add_utxos(&mut self, utxos: &Vec<HeritageUtxo>) -> Result<()>;

    /// Deletes [HeritageUtxo]s for the given [OutPoint]s
    ///
    /// If a UTXO doesn't exist for any given OutPoint, the operation
    /// will still succeed (idempotent deletion).
    fn delete_utxos(&mut self, outpoints: &Vec<OutPoint>) -> Result<()>;

    /// Returns all [HeritageUtxo]s from the database
    fn list_utxos(&self) -> Result<Vec<HeritageUtxo>>;
    /// Paginates the list of [HeritageUtxo]s from the database
    ///
    /// The caller **SHOULD NOT** assume that a page with fewer than `page_size`
    /// elements indicates the last page. Only the absence of a [ContinuationToken]
    /// in the returned [Paginated] struct indicates the final page.
    ///
    /// # Arguments
    ///
    /// * `page_size` - Maximum number of UTXOs to return per page
    /// * `continuation_token` - Token from previous page, or None for first page
    fn paginate_utxos(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<HeritageUtxo>>;

    /// Adds new TransactionSummaries ([TransactionSummary]) to the database
    ///
    /// Existing summaries with the same key will be overridden.
    fn add_transaction_summaries(
        &mut self,
        transaction_summaries: &Vec<TransactionSummary>,
    ) -> Result<()>;

    /// Deletes TransactionSummaries ([TransactionSummary]) for the given keys
    ///
    /// Keys are tuples of ([Txid], [Option<BlockTime>]). If a summary doesn't exist
    /// for any given key, the operation will still succeed (idempotent deletion).
    fn delete_transaction_summaries(
        &mut self,
        key_to_delete: &Vec<(Txid, Option<BlockTime>)>,
    ) -> Result<()>;
    /// Returns all TransactionSummaries ([TransactionSummary]) ordered by [BlockTime]
    ///
    /// Summaries are guaranteed to be ordered from newest to oldest by BlockTime.
    /// If two summaries share the same BlockTime, their relative order is undefined.
    fn list_transaction_summaries(&self) -> Result<Vec<TransactionSummary>>;
    /// Paginates the list of TransactionSummaries ([TransactionSummary]) from the database
    ///
    /// The caller **SHOULD NOT** assume that a page with fewer than `page_size`
    /// elements indicates the last page. Only the absence of a [ContinuationToken]
    /// in the returned [Paginated] struct indicates the final page.
    ///
    /// Summaries are guaranteed to be ordered from newest to oldest by [BlockTime].
    /// If two summaries share the same BlockTime, their relative order is undefined.
    ///
    /// # Arguments
    ///
    /// * `page_size` - Maximum number of summaries to return per page
    /// * `continuation_token` - Token from previous page, or None for first page
    fn paginate_transaction_summaries(
        &self,
        page_size: usize,
        continuation_token: Option<ContinuationToken>,
    ) -> Result<Paginated<TransactionSummary>>;

    /// Retrieves the current wallet balance ([HeritageWalletBalance]) from the database
    fn get_balance(&self) -> Result<Option<HeritageWalletBalance>>;

    /// Stores the wallet balance ([HeritageWalletBalance]) in the database
    fn set_balance(&mut self, new_balance: &HeritageWalletBalance) -> Result<()>;

    /// Retrieves the current fee rate ([FeeRate]) setting from the database
    fn get_fee_rate(&self) -> Result<Option<FeeRate>>;

    /// Stores the fee rate ([FeeRate]) setting in the database
    fn set_fee_rate(&mut self, new_fee_rate: &FeeRate) -> Result<()>;

    /// Retrieves the target block inclusion objective ([BlockInclusionObjective]) from the database
    ///
    /// This value is used to query the appropriate fee rate from Bitcoin Core
    /// based on how many blocks the user wants to wait for confirmation.
    fn get_block_inclusion_objective(&self) -> Result<Option<BlockInclusionObjective>>;

    /// Stores the block inclusion objective ([BlockInclusionObjective]) in the database
    ///
    /// This is used to query the appropriate fee rate from Bitcoin Core
    /// when creating transactions.
    fn set_block_inclusion_objective(
        &mut self,
        new_objective: BlockInclusionObjective,
    ) -> Result<()>;
}

/// Trait for databases that support transactional heritage operations
///
/// This trait extends HeritageDatabase with transaction support, allowing
/// multiple operations to be grouped and executed atomically.
pub trait TransacHeritageDatabase: HeritageDatabase {
    /// The transaction type that can accumulate operations
    type Transac: TransacHeritageOperation;

    /// Creates a new transaction container
    ///
    /// Operations can be added to the transaction before committing.
    fn begin_transac(&self) -> Self::Transac;

    /// Consumes and applies all operations in the transaction atomically
    ///
    /// # Errors
    ///
    /// Returns an error if any operation in the transaction fails,
    /// in which case the entire transaction is rolled back.
    fn commit_transac(&mut self, transac: Self::Transac) -> Result<()>;
}

#[cfg(any(test, feature = "database-tests"))]
pub mod tests {
    use core::str::FromStr;
    use std::collections::HashSet;

    use bdk::{
        database::{BatchOperations, Database},
        Balance, BlockTime, KeychainKind,
    };

    use crate::{
        bitcoin::{Amount, FeeRate, Txid},
        dbtests::{
            get_test_account_xpub, get_test_heritage_config, get_test_subwallet_config,
            TestHeritageConfig,
        },
        heritage_wallet::{TransactionSummaryIOTotals, TransactionSummaryOwnedIO},
    };

    use super::*;

    // Verify that we get the same database
    pub fn get_subdatabase<DB: TransacHeritageDatabase>(db: DB) {
        let subdb_index = SubdatabaseId("sub".to_owned());
        {
            let mut subdb = db.get_subdatabase(subdb_index.clone()).unwrap();
            // Should be empty, test that with get_last_index
            assert!(subdb
                .get_last_index(KeychainKind::External)
                .is_ok_and(|r| r.is_none()));
            // Insert stuff
            subdb.set_last_index(KeychainKind::External, 23).unwrap();
        } // Drop subdb
        let subdb = db.get_subdatabase(subdb_index).unwrap();
        // Should not be empty, test that with get_last_index
        assert!(subdb
            .get_last_index(KeychainKind::External)
            .is_ok_and(|r| r.is_some_and(|v| v == 23)));
    }

    // Verify that the transaction is either not executed or entirely executed
    pub fn transaction<DB: TransacHeritageDatabase>(mut db: DB) {
        // Prepare the database
        let axps = (0..10)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect();
        // Add AccountXPubs to the database
        db.add_unused_account_xpubs(&axps).unwrap();

        let swc1 = get_test_subwallet_config(0, TestHeritageConfig::BackupWifeY2);
        let swc2 = get_test_subwallet_config(1, TestHeritageConfig::BackupWifeY1);

        // Try to set the current subwallet_config with a wrong old_subwallet_config
        let mut bad_transac = db.begin_transac();
        bad_transac.delete_unused_account_xpub(&axps[0]).unwrap();
        bad_transac
            .safe_update_current_subwallet_config(&swc1, Some(&swc2))
            .unwrap();
        // Transac failled
        assert!(db.commit_transac(bad_transac).is_err());
        // DB content is the same
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Current)
            .is_ok_and(|r| r.is_none()));
        assert_eq!(db.list_unused_account_xpubs().unwrap(), axps);

        // Try to delete an unexisting AccountXPub
        let mut bad_transac = db.begin_transac();
        bad_transac
            .delete_unused_account_xpub(&get_test_account_xpub(10))
            .unwrap();
        bad_transac
            .safe_update_current_subwallet_config(&swc1, None)
            .unwrap();
        // Transac failled
        assert!(db.commit_transac(bad_transac).is_err());
        // DB content is the same
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Current)
            .is_ok_and(|r| r.is_none()));
        assert_eq!(db.list_unused_account_xpubs().unwrap(), axps);

        // Valid transaction
        let mut good_transac = db.begin_transac();
        good_transac.delete_unused_account_xpub(&axps[0]).unwrap();
        good_transac
            .safe_update_current_subwallet_config(&swc1, None)
            .unwrap();
        // Transac success
        let res = db.commit_transac(good_transac);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // DB content is updated
        assert_eq!(db.list_unused_account_xpubs().unwrap(), &axps[1..10]);
        assert_eq!(db.list_used_account_xpubs().unwrap(), &axps[0..1]);
        assert_eq!(
            db.get_subwallet_config(SubwalletConfigId::Current)
                .unwrap()
                .unwrap(),
            swc1
        );

        // Try to insert an existing index
        let mut bad_transac = db.begin_transac();
        bad_transac.delete_unused_account_xpub(&axps[1]).unwrap();
        bad_transac
            .safe_update_current_subwallet_config(&swc2, Some(&swc1))
            .unwrap();
        bad_transac
            .put_subwallet_config(SubwalletConfigId::Current, &swc1)
            .unwrap();
        // Transac failled
        assert!(db.commit_transac(bad_transac).is_err());
        // DB content is the same
        assert_eq!(db.list_unused_account_xpubs().unwrap(), &axps[1..10]);
        assert_eq!(db.list_used_account_xpubs().unwrap(), &axps[0..1]);
        assert_eq!(
            db.get_subwallet_config(SubwalletConfigId::Current)
                .unwrap()
                .unwrap(),
            swc1
        );

        // Valid transaction
        let mut good_transac = db.begin_transac();
        good_transac.delete_unused_account_xpub(&axps[1]).unwrap();
        good_transac
            .safe_update_current_subwallet_config(&swc2, Some(&swc1))
            .unwrap();
        good_transac
            .put_subwallet_config(SubwalletConfigId::Id(0), &swc1)
            .unwrap();
        // Transac success
        let res = db.commit_transac(good_transac);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // DB content is updated
        assert_eq!(db.list_unused_account_xpubs().unwrap(), &axps[2..10]);
        assert_eq!(db.list_used_account_xpubs().unwrap(), &axps[0..2]);
        assert_eq!(
            db.get_subwallet_config(SubwalletConfigId::Current)
                .unwrap()
                .unwrap(),
            swc2
        );
        assert_eq!(db.list_obsolete_subwallet_configs().unwrap(), &[swc1]);
    }

    // Verify that we cannot override an index
    pub fn get_put_subwallet_config<DB: TransacHeritageDatabase>(mut db: DB) {
        let swc1 = get_test_subwallet_config(0, TestHeritageConfig::BackupWifeY1);
        let swc2 = get_test_subwallet_config(1, TestHeritageConfig::BackupWifeBro);
        // Nothing in the database
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Current)
            .is_ok_and(|r| r.is_none()));
        // We can insert
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Current, &swc1)
            .is_ok());
        // Something in the database
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Current)
            .is_ok_and(|r| r.is_some_and(|swc| swc == swc1)));
        // We cannot override
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Current, &swc1)
            .is_err());
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Current, &swc2)
            .is_err());
        // Nothing in the database
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Id(0))
            .is_ok_and(|r| r.is_none()));
        // We can insert
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Id(0), &swc1)
            .is_ok());
        // Something in the database
        assert!(db
            .get_subwallet_config(SubwalletConfigId::Id(0))
            .is_ok_and(|r| r.is_some_and(|swc| swc == swc1)));
        // We cannot override
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Id(0), &swc2)
            .is_err());
        // We can insert another index
        assert!(db
            .put_subwallet_config(SubwalletConfigId::Id(1), &swc2)
            .is_ok());
    }

    // Verify that we can update only if the current wallet is expected
    pub fn safe_update_current_subwallet_config<DB: TransacHeritageDatabase>(mut db: DB) {
        let swc1 = get_test_subwallet_config(0, TestHeritageConfig::BackupWifeY1);
        let swc2 = get_test_subwallet_config(1, TestHeritageConfig::BackupWifeBro);
        // DB is empty so expecting a previous value fails
        assert!(db
            .safe_update_current_subwallet_config(&swc1, Some(&swc2))
            .is_err());
        // Expecting None allow the insertion
        let res = db.safe_update_current_subwallet_config(&swc1, None);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // DB is now filled, expecting None fails
        assert!(db
            .safe_update_current_subwallet_config(&swc2, None)
            .is_err());
        // Expecting the wrong value fails
        assert!(db
            .safe_update_current_subwallet_config(&swc2, Some(&swc2))
            .is_err());
        // Expecting the correct value succeed
        let res = db.safe_update_current_subwallet_config(&swc2, Some(&swc1));
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
    }

    pub fn unused_account_xpub_management<DB: TransacHeritageDatabase>(mut db: DB) {
        // At this point, no AccountXPubs
        let res = db.list_unused_account_xpubs();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_empty());

        // Retrieve some individual test ads (0, 1 and 11)
        let axp0 = get_test_account_xpub(0);
        let axp1 = get_test_account_xpub(1);
        let axp11 = get_test_account_xpub(11);

        // List of all test Ads from 0 to 9 included
        let axps = (0..10)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect();
        // Add it to the database
        db.add_unused_account_xpubs(&axps).unwrap();

        // List account descriptors should now give us what we just added
        let res = db.list_unused_account_xpubs();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert_eq!(res.unwrap(), axps);

        // Deleting the ad11 fails because it does not exist
        assert!(db.delete_unused_account_xpub(&axp11).is_err());
        // Deleting the adO works
        let res = db.delete_unused_account_xpub(&axp0);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Deleting the adO fails because it does not exist anymore
        assert!(db.delete_unused_account_xpub(&axp0).is_err());
        // Deleting the ad1 works
        let res = db.delete_unused_account_xpub(&axp1);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // At this point, no used AccountXPubs
        let res = db.list_used_account_xpubs();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_empty());

        // Using ad0/ad1 for a SubwalletConfig
        let subwallet_config = get_test_subwallet_config(0, TestHeritageConfig::BackupWifeBro);
        db.put_subwallet_config(SubwalletConfigId::Id(1), &subwallet_config)
            .unwrap();
        let subwallet_config = get_test_subwallet_config(1, TestHeritageConfig::BackupWifeBro);
        db.put_subwallet_config(SubwalletConfigId::Current, &subwallet_config)
            .unwrap();
        // Deleting the adO fails because it is not "unused" anymore
        assert!(db.delete_unused_account_xpub(&axp0).is_err());
        // Deleting the ad1 fails because it is not "unused" anymore
        assert!(db.delete_unused_account_xpub(&axp1).is_err());

        // Should be returned by list_used_account_xpubs
        let res = db.list_used_account_xpubs();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert_eq!(res.unwrap(), vec![axp0, axp1]);

        // Adding AccountXPubs 0..15
        // As 0 and 1 are used, they should be filtered out
        let axps = (0..15)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect();
        db.add_unused_account_xpubs(&axps).unwrap();
        let expect = (2..15)
            .into_iter()
            .map(|i| get_test_account_xpub(i))
            .collect::<Vec<_>>();
        // List account descriptors
        let ret = db.list_unused_account_xpubs().unwrap();
        assert_eq!(ret, expect);

        // At this point, get_unused_account_xpub should give us a value
        let unused_axp = db.get_unused_account_xpub().unwrap().unwrap();
        // That belong to the collection
        assert!(expect.contains(&unused_axp));
        // Remove it, we should have something else
        db.delete_unused_account_xpub(&unused_axp).unwrap();
        let unused_axp2 = db.get_unused_account_xpub().unwrap().unwrap();
        assert_ne!(unused_axp, unused_axp2);
    }

    pub fn heritage_utxo_management<DB: TransacHeritageDatabase>(mut db: DB) {
        // At this point, no HeritageUtxo
        let res = db.list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_empty());

        let heritage_utxo_1 = HeritageUtxo {
            outpoint: OutPoint::from_str(
                "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:0",
            )
            .unwrap(),
            amount: Amount::from_sat(10_000),
            confirmation_time: Some(BlockTime {
                height: 123_456,
                timestamp: 1_700_000_000,
            }),
            address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                .try_into()
                .unwrap(),
            heritage_config: get_test_heritage_config(TestHeritageConfig::BackupWifeBro),
        };
        let heritage_utxo_2 = HeritageUtxo {
            outpoint: OutPoint::from_str(
                "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:1",
            )
            .unwrap(),
            amount: Amount::from_sat(10_000),
            confirmation_time: Some(BlockTime {
                height: 123_456,
                timestamp: 1_700_000_000,
            }),
            address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                .try_into()
                .unwrap(),
            heritage_config: get_test_heritage_config(TestHeritageConfig::BackupWifeBro),
        };
        let heritage_utxo_3 = HeritageUtxo {
            outpoint: OutPoint::from_str(
                "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:2",
            )
            .unwrap(),
            amount: Amount::from_sat(10_000),
            confirmation_time: Some(BlockTime {
                height: 123_456,
                timestamp: 1_700_000_000,
            }),
            address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                .try_into()
                .unwrap(),
            heritage_config: get_test_heritage_config(TestHeritageConfig::BackupWifeBro),
        };

        // Add two UTXO
        let to_add1 = vec![heritage_utxo_1.clone(), heritage_utxo_2.clone()];
        let res = db.add_utxos(&to_add1);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List Utxo should give us 2
        let res = db.list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        assert_eq!(res.unwrap().len(), 2);

        // Add UTXO with overlap
        let to_add2 = vec![
            heritage_utxo_1.clone(),
            heritage_utxo_3.clone(),
            heritage_utxo_2.clone(),
        ];
        let res = db.add_utxos(&to_add2);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List Utxo should give us 3
        let res = db.list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        let lst1 = res.unwrap();
        assert_eq!(lst1.len(), 3);

        // Paginate Utxo should give us the same result
        let mut lst2 = vec![];
        let mut continuation_token = None;
        loop {
            let res = db.paginate_utxos(1, continuation_token);
            assert!(res.is_ok(), "{:#}", res.unwrap_err());
            let res = res.unwrap();
            let is_last_page = res.is_last_page();
            let mut page = res.page;
            lst2.append(&mut page);
            if is_last_page {
                break;
            }
            continuation_token = res.continuation_token;
        }
        assert_eq!(lst1, lst2);

        // Remove UTXOs
        let to_remove = to_add1.iter().map(|utxo| utxo.outpoint).collect();
        let res = db.delete_utxos(&to_remove);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List Utxo should give us 1 and it should be heritage_utxo_3
        let res = db.list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        let res = res.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].outpoint, heritage_utxo_3.outpoint);

        // Re-remove should not do anything at all
        let res = db.delete_utxos(&to_remove);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List Utxo should give us 1 and it should be heritage_utxo_3
        let res = db.list_utxos();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        let res = res.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].outpoint, heritage_utxo_3.outpoint);
    }

    pub fn transaction_summaries_management<DB: TransacHeritageDatabase>(mut db: DB) {
        // At this point, no TransactionSummary
        let res = db.list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_empty());

        let txid =
            Txid::from_str("5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456")
                .unwrap();
        let tx_summary_1 = TransactionSummary {
            txid,
            confirmation_time: Some(BlockTime {
                height: 123_455,
                timestamp: 1_700_000_000,
            }),
            owned_inputs: vec![TransactionSummaryOwnedIO {
                outpoint: OutPoint {
                    txid: Txid::from_str(
                        "5df6e0e2761359d30a8275058d765fcc0381534545f55cf43e41983f5d4c9456",
                    )
                    .unwrap(),
                    vout: 1,
                },
                address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                    .try_into()
                    .unwrap(),
                amount: Amount::from_sat(100_000),
            }],
            inputs_totals: TransactionSummaryIOTotals {
                count: 1,
                amount: Amount::from_sat(100_000),
            },
            owned_outputs: vec![],
            outputs_totals: TransactionSummaryIOTotals {
                count: 1,
                amount: Amount::from_sat(90_000),
            },
            fee: Amount::from_sat(10_000),
            fee_rate: FeeRate::from_sat_per_vb_unchecked(3),
            parent_txids: HashSet::new(),
        };
        let txid =
            Txid::from_str("5df6e0e2761359d30a8275058e300fcc0381534545f55cf43e41983f5d4c9456")
                .unwrap();
        let tx_summary_2 = TransactionSummary {
            txid,
            confirmation_time: Some(BlockTime {
                height: 123_452,
                timestamp: 1_700_000_000,
            }),
            owned_inputs: vec![],
            inputs_totals: TransactionSummaryIOTotals {
                count: 1,
                amount: Amount::from_sat(200_000),
            },
            owned_outputs: vec![TransactionSummaryOwnedIO {
                outpoint: OutPoint { txid, vout: 0 },
                address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                    .try_into()
                    .unwrap(),
                amount: Amount::from_sat(100_000),
            }],
            outputs_totals: TransactionSummaryIOTotals {
                count: 2,
                amount: Amount::from_sat(190_000),
            },
            fee: Amount::from_sat(10_000),
            fee_rate: FeeRate::from_sat_per_vb_unchecked(3),
            parent_txids: HashSet::new(),
        };
        let txid =
            Txid::from_str("5df6e0e2761359d30a8275058e201fcc0381534545f55cf43e41983f5d4c9456")
                .unwrap();
        let tx_summary_3 = TransactionSummary {
            txid,
            confirmation_time: Some(BlockTime {
                height: 123_455,
                timestamp: 1_700_000_000,
            }),
            owned_inputs: vec![TransactionSummaryOwnedIO {
                outpoint: OutPoint {
                    txid: Txid::from_str(
                        "5df6e0e2761359d30a8275058d765fcc0381534545f55cf43e41983f5d4c9456",
                    )
                    .unwrap(),
                    vout: 1,
                },
                address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                    .try_into()
                    .unwrap(),
                amount: Amount::from_sat(100_000),
            }],
            inputs_totals: TransactionSummaryIOTotals {
                count: 1,
                amount: Amount::from_sat(100_000),
            },
            owned_outputs: vec![TransactionSummaryOwnedIO {
                outpoint: OutPoint { txid, vout: 0 },
                address: "bcrt1p30dak2tfa6m7erhayrmmceykrfmqxy6qf6gqzzdphgv6lw9s9ykq4w70ya"
                    .try_into()
                    .unwrap(),
                amount: Amount::from_sat(90_000),
            }],
            outputs_totals: TransactionSummaryIOTotals {
                count: 1,
                amount: Amount::from_sat(90_000),
            },
            fee: Amount::from_sat(10_000),
            fee_rate: FeeRate::from_sat_per_vb_unchecked(3),
            parent_txids: HashSet::from([Txid::from_str(
                "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456",
            )
            .unwrap()]),
        };

        // Add two TransactionSummary
        let to_add1 = vec![tx_summary_1.clone(), tx_summary_2.clone()];
        let res = db.add_transaction_summaries(&to_add1);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List TransactionSummary should give us 2
        let res = db.list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        assert_eq!(res.unwrap().len(), 2);

        // Add TransactionSummary with overlap
        let to_add2 = vec![
            tx_summary_1.clone(),
            tx_summary_3.clone(),
            tx_summary_2.clone(),
        ];
        let res = db.add_transaction_summaries(&to_add2);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List TransactionSummary should give us 3
        let res = db.list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        let lst1 = res.unwrap();
        assert_eq!(lst1.len(), 3);

        // Paginate TransactionSummary should give us the same result
        let mut lst2 = vec![];
        let mut continuation_token = None;
        loop {
            let res = db.paginate_transaction_summaries(1, continuation_token);
            assert!(res.is_ok(), "{:#}", res.unwrap_err());
            let res = res.unwrap();
            let is_last_page = res.is_last_page();
            let mut page = res.page;
            lst2.append(&mut page);
            if is_last_page {
                break;
            }
            continuation_token = res.continuation_token;
        }
        // Sorting is guaranteed by the database API only for TX sum with different heights
        // So the last TX must be tx_summary_2 (because it has the lowest, unique block height)
        assert_eq!(lst1.last().unwrap(), &tx_summary_2);
        assert_eq!(lst2.last().unwrap(), &tx_summary_2);
        // Verify the two lists have the same elems
        assert!(
            lst1.len() == lst2.len()
                && lst1.iter().all(|e| lst2.contains(e))
                && lst2.iter().all(|e| lst1.contains(e))
        );

        // Remove TransactionSummary
        let to_delete = to_add1
            .into_iter()
            .map(|txsum| (txsum.txid, txsum.confirmation_time))
            .collect();
        let res = db.delete_transaction_summaries(&to_delete);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List TransactionSummary should give us 1 and it should be tx_summary_3
        let res = db.list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        let res = res.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].txid, tx_summary_3.txid);

        // Re-remove should not do anything at all
        let res = db.delete_transaction_summaries(&to_delete);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());

        // List TransactionSummary should give us 1 and it should be tx_summary_3
        let res = db.list_transaction_summaries();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // We do not test the Vec equalities because they are may not be in the same order
        let res = res.unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].txid, tx_summary_3.txid);
    }

    pub fn get_set_balance<DB: TransacHeritageDatabase>(mut db: DB) {
        // Get balance works and is None
        let res = db.get_balance();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_none());

        let balance = HeritageWalletBalance::default();
        // Insert work
        let res = db.set_balance(&balance);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get balance return the inserted Balance
        let res = db.get_balance();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|b| b == balance));

        let balance = HeritageWalletBalance::new(
            Balance {
                immature: 10,
                trusted_pending: 0,
                untrusted_pending: 0,
                confirmed: 1000,
            },
            Balance::default(),
        );
        // Update works
        let res = db.set_balance(&balance);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get balance return the updated Balance
        let res = db.get_balance();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|b| b == balance));
    }

    pub fn get_set_fee_rate<DB: TransacHeritageDatabase>(mut db: DB) {
        // Get FeeRate works and is None
        let res = db.get_fee_rate();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_none());

        let fee_rate = FeeRate::from_sat_per_vb_unchecked(10);
        // Insert work
        let res = db.set_fee_rate(&fee_rate);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get FeeRate return the inserted FeeRate
        let res = db.get_fee_rate();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|fr| fr == fee_rate));

        let fee_rate = FeeRate::from_sat_per_vb_unchecked(5);
        // Update works
        let res = db.set_fee_rate(&fee_rate);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get FeeRate return the updated FeeRate
        let res = db.get_fee_rate();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|fr| fr == fee_rate));
    }

    pub fn get_set_block_inclusion_objective<DB: TransacHeritageDatabase>(mut db: DB) {
        // Get bio works and is None
        let res = db.get_block_inclusion_objective();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_none());

        let new_bio = BlockInclusionObjective::from(5u16);
        // Insert work
        let res = db.set_block_inclusion_objective(new_bio);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get bio return the inserted bio
        let res = db.get_block_inclusion_objective();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|bio| bio == new_bio));

        let new_bio = BlockInclusionObjective::from(10u16);
        // Update works
        let res = db.set_block_inclusion_objective(new_bio);
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        // Get bio return the updated bio
        let res = db.get_block_inclusion_objective();
        assert!(res.is_ok(), "{:#}", res.unwrap_err());
        assert!(res.unwrap().is_some_and(|bio| bio == new_bio));
    }

    pub fn list_obsolete_subwallet_configs<DB: TransacHeritageDatabase>(mut db: DB) {
        let subwallet_config0 = get_test_subwallet_config(0, TestHeritageConfig::BackupWifeBro);
        db.put_subwallet_config(SubwalletConfigId::Id(0), &subwallet_config0)
            .unwrap();
        let subwallet_config1 = get_test_subwallet_config(1, TestHeritageConfig::BackupWifeBro);
        db.put_subwallet_config(SubwalletConfigId::Id(1), &subwallet_config1)
            .unwrap();
        let subwallet_config2 = get_test_subwallet_config(2, TestHeritageConfig::BackupWifeBro);
        db.put_subwallet_config(SubwalletConfigId::Current, &subwallet_config2)
            .unwrap();

        // obsolete subwallet config : 0 and 1
        let obsolete = db.list_obsolete_subwallet_configs();
        assert!(obsolete.is_ok(), "{:#}", obsolete.unwrap_err());
        let obsolete = obsolete.unwrap();
        assert_eq!(obsolete, vec![subwallet_config0, subwallet_config1]);
    }
}

#[cfg(any(test, feature = "database-tests"))]
pub mod bdk_tests {
    use bdk::{
        bitcoin::{
            consensus::{encode::deserialize, serialize},
            hashes::hex::*,
            *,
        },
        database::{BatchOperations, Database, SyncTime},
        BlockTime, KeychainKind, LocalUtxo, TransactionDetails,
    };
    use core::str::FromStr;

    use super::*;

    pub fn test_script_pubkey<D: Database>(mut db: D) {
        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;
        let keychain = KeychainKind::External;

        db.set_script_pubkey(&script, keychain, path).unwrap();

        assert_eq!(
            db.get_script_pubkey_from_path(keychain, path).unwrap(),
            Some(script.clone())
        );
        assert_eq!(
            db.get_path_from_script_pubkey(&script).unwrap(),
            Some((keychain, path))
        );
    }

    pub fn test_batch_script_pubkey<D: BatchDatabase>(mut db: D) {
        let mut batch = db.begin_batch();

        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;
        let keychain = KeychainKind::External;

        batch.set_script_pubkey(&script, keychain, path).unwrap();

        assert_eq!(
            db.get_script_pubkey_from_path(keychain, path).unwrap(),
            None
        );
        assert_eq!(db.get_path_from_script_pubkey(&script).unwrap(), None);

        db.commit_batch(batch).unwrap();

        assert_eq!(
            db.get_script_pubkey_from_path(keychain, path).unwrap(),
            Some(script.clone())
        );
        assert_eq!(
            db.get_path_from_script_pubkey(&script).unwrap(),
            Some((keychain, path))
        );
    }

    pub fn test_iter_script_pubkey<D: Database>(mut db: D) {
        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;
        let keychain = KeychainKind::External;

        db.set_script_pubkey(&script, keychain, path).unwrap();

        assert_eq!(db.iter_script_pubkeys(None).unwrap().len(), 1);
    }

    pub fn test_del_script_pubkey<D: Database>(mut db: D) {
        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;
        let keychain = KeychainKind::External;

        db.set_script_pubkey(&script, keychain, path).unwrap();
        assert_eq!(db.iter_script_pubkeys(None).unwrap().len(), 1);

        db.del_script_pubkey_from_path(keychain, path).unwrap();
        assert_eq!(db.iter_script_pubkeys(None).unwrap().len(), 0);
    }

    pub fn test_utxo<D: Database>(mut db: D) {
        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:0",
        )
        .unwrap();
        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let txout = TxOut {
            value: 133742,
            script_pubkey: script,
        };
        let utxo = LocalUtxo {
            txout,
            outpoint,
            keychain: KeychainKind::External,
            is_spent: true,
        };

        db.set_utxo(&utxo).unwrap();
        db.set_utxo(&utxo).unwrap();
        assert_eq!(db.iter_utxos().unwrap().len(), 1);
        assert_eq!(db.get_utxo(&outpoint).unwrap(), Some(utxo));
    }

    pub fn test_raw_tx<D: Database>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("02000000000101f58c18a90d7a76b30c7e47d4e817adfdd79a6a589a615ef36e360f913adce2cd0000000000feffffff0210270000000000001600145c9a1816d38db5cbdd4b067b689dc19eb7d930e2cf70aa2b080000001600140f48b63160043047f4f60f7f8f551f80458f693f024730440220413f42b7bc979945489a38f5221e5527d4b8e3aa63eae2099e01945896ad6c10022024ceec492d685c31d8adb64e935a06933877c5ae0e21f32efe029850914c5bad012102361caae96f0e9f3a453d354bb37a5c3244422fb22819bf0166c0647a38de39f21fca2300").unwrap();
        let mut tx: Transaction = deserialize(&hex_tx).unwrap();

        db.set_raw_tx(&tx).unwrap();

        let txid = tx.txid();

        assert_eq!(db.get_raw_tx(&txid).unwrap(), Some(tx.clone()));

        // mutate transaction's witnesses
        for tx_in in tx.input.iter_mut() {
            tx_in.witness = Witness::new();
        }

        let updated_hex_tx = serialize(&tx);

        // verify that mutation was successful
        assert_ne!(hex_tx, updated_hex_tx);

        db.set_raw_tx(&tx).unwrap();

        let txid = tx.txid();

        assert_eq!(db.get_raw_tx(&txid).unwrap(), Some(tx));
    }

    pub fn test_batch_raw_tx<D: BatchDatabase>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("02000000000101f58c18a90d7a76b30c7e47d4e817adfdd79a6a589a615ef36e360f913adce2cd0000000000feffffff0210270000000000001600145c9a1816d38db5cbdd4b067b689dc19eb7d930e2cf70aa2b080000001600140f48b63160043047f4f60f7f8f551f80458f693f024730440220413f42b7bc979945489a38f5221e5527d4b8e3aa63eae2099e01945896ad6c10022024ceec492d685c31d8adb64e935a06933877c5ae0e21f32efe029850914c5bad012102361caae96f0e9f3a453d354bb37a5c3244422fb22819bf0166c0647a38de39f21fca2300").unwrap();
        let mut tx: Transaction = deserialize(&hex_tx).unwrap();

        let mut batch = db.begin_batch();
        batch.set_raw_tx(&tx).unwrap();

        let txid = tx.txid();
        // Batch not yet comited
        assert_eq!(db.get_raw_tx(&txid).unwrap(), None);

        db.commit_batch(batch).unwrap();
        // Batch comited
        assert_eq!(db.get_raw_tx(&txid).unwrap(), Some(tx.clone()));

        let initial_tx = tx.clone();
        // mutate transaction's witnesses
        for tx_in in tx.input.iter_mut() {
            tx_in.witness = Witness::new();
        }

        let updated_hex_tx = serialize(&tx);

        // verify that mutation was successful
        assert_ne!(hex_tx, updated_hex_tx);

        let mut batch = db.begin_batch();
        batch.set_raw_tx(&tx).unwrap();

        // Batch not yet comited
        assert_eq!(db.get_raw_tx(&txid).unwrap(), Some(initial_tx));

        db.commit_batch(batch).unwrap();

        // Batch comited
        assert_eq!(db.get_raw_tx(&txid).unwrap(), Some(tx));
    }

    pub fn test_tx<D: Database>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let tx: Transaction = deserialize(&hex_tx).unwrap();
        let txid = tx.txid();
        let mut tx_details = TransactionDetails {
            transaction: Some(tx),
            txid,
            received: 1337,
            sent: 420420,
            fee: Some(140),
            confirmation_time: Some(BlockTime {
                timestamp: 123456,
                height: 1000,
            }),
        };

        db.set_tx(&tx_details).unwrap();

        // get with raw tx too
        assert_eq!(
            db.get_tx(&tx_details.txid, true).unwrap(),
            Some(tx_details.clone())
        );
        // get only raw_tx
        assert_eq!(
            db.get_raw_tx(&tx_details.txid).unwrap(),
            tx_details.transaction
        );

        // now get without raw_tx
        tx_details.transaction = None;
        assert_eq!(
            db.get_tx(&tx_details.txid, false).unwrap(),
            Some(tx_details)
        );
    }

    pub fn test_batch_tx<D: BatchDatabase>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let tx: Transaction = deserialize(&hex_tx).unwrap();
        let txid = tx.txid();
        let mut tx_details = TransactionDetails {
            transaction: Some(tx),
            txid,
            received: 1337,
            sent: 420420,
            fee: Some(140),
            confirmation_time: Some(BlockTime {
                timestamp: 123456,
                height: 1000,
            }),
        };
        let mut batch = db.begin_batch();
        batch.set_tx(&tx_details).unwrap();

        // Batch not comited
        // get with raw tx too
        assert_eq!(db.get_tx(&tx_details.txid, true).unwrap(), None);
        // get only raw_tx
        assert_eq!(db.get_raw_tx(&tx_details.txid).unwrap(), None);
        // now get without raw_tx
        assert_eq!(db.get_tx(&tx_details.txid, false).unwrap(), None);

        db.commit_batch(batch).unwrap();

        // Batch comited
        // get with raw tx too
        assert_eq!(
            db.get_tx(&tx_details.txid, true).unwrap(),
            Some(tx_details.clone())
        );
        // get only raw_tx
        assert_eq!(
            db.get_raw_tx(&tx_details.txid).unwrap(),
            tx_details.transaction
        );
        // now get without raw_tx
        tx_details.transaction = None;
        assert_eq!(
            db.get_tx(&tx_details.txid, false).unwrap(),
            Some(tx_details)
        );
    }

    pub fn test_list_transaction<D: Database>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let tx: Transaction = deserialize(&hex_tx).unwrap();
        let txid = tx.txid();
        let mut tx_details = TransactionDetails {
            transaction: Some(tx),
            txid,
            received: 1337,
            sent: 420420,
            fee: Some(140),
            confirmation_time: Some(BlockTime {
                timestamp: 123456,
                height: 1000,
            }),
        };

        db.set_tx(&tx_details).unwrap();

        // get raw tx
        assert_eq!(db.iter_txs(true).unwrap(), vec![tx_details.clone()]);

        // now get without raw tx
        tx_details.transaction = None;

        // get not raw tx
        assert_eq!(db.iter_txs(false).unwrap(), vec![tx_details.clone()]);
    }

    pub fn test_last_index<D: Database>(mut db: D) {
        db.set_last_index(KeychainKind::External, 1337).unwrap();

        assert_eq!(
            db.get_last_index(KeychainKind::External).unwrap(),
            Some(1337)
        );
        assert_eq!(db.get_last_index(KeychainKind::Internal).unwrap(), None);

        let res = db.increment_last_index(KeychainKind::External).unwrap();
        assert_eq!(res, 1338);
        let res = db.increment_last_index(KeychainKind::Internal).unwrap();
        assert_eq!(res, 0);

        assert_eq!(
            db.get_last_index(KeychainKind::External).unwrap(),
            Some(1338)
        );
        assert_eq!(db.get_last_index(KeychainKind::Internal).unwrap(), Some(0));
    }

    pub fn test_sync_time<D: Database>(mut db: D) {
        assert!(db.get_sync_time().unwrap().is_none());

        db.set_sync_time(SyncTime {
            block_time: BlockTime {
                height: 100,
                timestamp: 1000,
            },
        })
        .unwrap();

        let extracted = db.get_sync_time().unwrap();
        assert!(extracted.is_some());
        assert_eq!(extracted.as_ref().unwrap().block_time.height, 100);
        assert_eq!(extracted.as_ref().unwrap().block_time.timestamp, 1000);

        db.del_sync_time().unwrap();
        assert!(db.get_sync_time().unwrap().is_none());
    }

    pub fn test_iter_raw_txs<D: Database>(mut db: D) {
        let txs = db.iter_raw_txs().unwrap();
        assert!(txs.is_empty());

        let hex_tx = Vec::<u8>::from_hex("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let first_tx: Transaction = deserialize(&hex_tx).unwrap();

        let hex_tx = Vec::<u8>::from_hex("02000000000101f58c18a90d7a76b30c7e47d4e817adfdd79a6a589a615ef36e360f913adce2cd0000000000feffffff0210270000000000001600145c9a1816d38db5cbdd4b067b689dc19eb7d930e2cf70aa2b080000001600140f48b63160043047f4f60f7f8f551f80458f693f024730440220413f42b7bc979945489a38f5221e5527d4b8e3aa63eae2099e01945896ad6c10022024ceec492d685c31d8adb64e935a06933877c5ae0e21f32efe029850914c5bad012102361caae96f0e9f3a453d354bb37a5c3244422fb22819bf0166c0647a38de39f21fca2300").unwrap();
        let second_tx: Transaction = deserialize(&hex_tx).unwrap();

        db.set_raw_tx(&first_tx).unwrap();
        db.set_raw_tx(&second_tx).unwrap();

        let txs = db.iter_raw_txs().unwrap();

        assert!(txs.contains(&first_tx));
        assert!(txs.contains(&second_tx));
        assert_eq!(txs.len(), 2);
    }

    pub fn test_del_path_from_script_pubkey<D: Database>(mut db: D) {
        let keychain = KeychainKind::External;

        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;

        let res = db.del_path_from_script_pubkey(&script).unwrap();

        assert!(res.is_none());

        let _res = db.set_script_pubkey(&script, keychain, path);
        let (chain, child) = db.del_path_from_script_pubkey(&script).unwrap().unwrap();

        assert_eq!(chain, keychain);
        assert_eq!(child, path);

        let res = db.get_path_from_script_pubkey(&script).unwrap();
        assert!(res.is_none());
    }

    pub fn test_iter_script_pubkeys<D: Database>(mut db: D) {
        let keychain = KeychainKind::External;
        let scripts = db.iter_script_pubkeys(Some(keychain)).unwrap();
        assert!(scripts.is_empty());

        let first_script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let path = 42;

        db.set_script_pubkey(&first_script, keychain, path).unwrap();

        let second_script = ScriptBuf::from(
            Vec::<u8>::from_hex("00145c9a1816d38db5cbdd4b067b689dc19eb7d930e2").unwrap(),
        );
        let path = 57;

        db.set_script_pubkey(&second_script, keychain, path)
            .unwrap();
        let scripts = db.iter_script_pubkeys(Some(keychain)).unwrap();

        assert!(scripts.contains(&first_script));
        assert!(scripts.contains(&second_script));
        assert_eq!(scripts.len(), 2);
    }

    pub fn test_del_utxo<D: Database>(mut db: D) {
        let outpoint = OutPoint::from_str(
            "5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456:0",
        )
        .unwrap();
        let script = ScriptBuf::from(
            Vec::<u8>::from_hex("76a91402306a7c23f3e8010de41e9e591348bb83f11daa88ac").unwrap(),
        );
        let txout = TxOut {
            value: 133742,
            script_pubkey: script,
        };
        let utxo = LocalUtxo {
            txout,
            outpoint,
            keychain: KeychainKind::External,
            is_spent: true,
        };

        let res = db.del_utxo(&outpoint).unwrap();
        assert!(res.is_none());

        db.set_utxo(&utxo).unwrap();

        let res = db.del_utxo(&outpoint).unwrap();

        assert_eq!(res.unwrap(), utxo);

        let res = db.get_utxo(&outpoint).unwrap();
        assert!(res.is_none());
    }

    pub fn test_del_raw_tx<D: Database>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("02000000000101f58c18a90d7a76b30c7e47d4e817adfdd79a6a589a615ef36e360f913adce2cd0000000000feffffff0210270000000000001600145c9a1816d38db5cbdd4b067b689dc19eb7d930e2cf70aa2b080000001600140f48b63160043047f4f60f7f8f551f80458f693f024730440220413f42b7bc979945489a38f5221e5527d4b8e3aa63eae2099e01945896ad6c10022024ceec492d685c31d8adb64e935a06933877c5ae0e21f32efe029850914c5bad012102361caae96f0e9f3a453d354bb37a5c3244422fb22819bf0166c0647a38de39f21fca2300").unwrap();
        let tx: Transaction = deserialize(&hex_tx).unwrap();

        let res = db.del_raw_tx(&tx.txid()).unwrap();

        assert!(res.is_none());

        db.set_raw_tx(&tx).unwrap();

        let res = db.del_raw_tx(&tx.txid()).unwrap();

        assert_eq!(res.unwrap(), tx);

        let res = db.get_raw_tx(&tx.txid()).unwrap();
        assert!(res.is_none());
    }

    pub fn test_del_tx<D: Database>(mut db: D) {
        let hex_tx = Vec::<u8>::from_hex("0100000001a15d57094aa7a21a28cb20b59aab8fc7d1149a3bdbcddba9c622e4f5f6a99ece010000006c493046022100f93bb0e7d8db7bd46e40132d1f8242026e045f03a0efe71bbb8e3f475e970d790221009337cd7f1f929f00cc6ff01f03729b069a7c21b59b1736ddfee5db5946c5da8c0121033b9b137ee87d5a812d6f506efdd37f0affa7ffc310711c06c7f3e097c9447c52ffffffff0100e1f505000000001976a9140389035a9225b3839e2bbf32d826a1e222031fd888ac00000000").unwrap();
        let tx: Transaction = deserialize(&hex_tx).unwrap();
        let txid = tx.txid();
        let mut tx_details = TransactionDetails {
            transaction: Some(tx.clone()),
            txid,
            received: 1337,
            sent: 420420,
            fee: Some(140),
            confirmation_time: Some(BlockTime {
                timestamp: 123456,
                height: 1000,
            }),
        };

        let res = db.del_tx(&tx.txid(), true).unwrap();

        assert!(res.is_none());

        db.set_tx(&tx_details).unwrap();

        let res = db.del_tx(&tx.txid(), false).unwrap();
        tx_details.transaction = None;
        assert_eq!(res.unwrap(), tx_details);

        let res = db.get_tx(&tx.txid(), true).unwrap();
        assert!(res.is_none());

        let res = db.get_raw_tx(&tx.txid()).unwrap();
        assert_eq!(res.unwrap(), tx);

        db.set_tx(&tx_details).unwrap();
        let res = db.del_tx(&tx.txid(), true).unwrap();
        tx_details.transaction = Some(tx.clone());
        assert_eq!(res.unwrap(), tx_details);

        let res = db.get_tx(&tx.txid(), true).unwrap();
        assert!(res.is_none());

        let res = db.get_raw_tx(&tx.txid()).unwrap();
        assert!(res.is_none());
    }

    pub fn test_del_last_index<D: Database>(mut db: D) {
        let keychain = KeychainKind::External;

        let _res = db.increment_last_index(keychain);

        let res = db.get_last_index(keychain).unwrap().unwrap();

        assert_eq!(res, 0);

        let _res = db.increment_last_index(keychain);

        let res = db.del_last_index(keychain).unwrap().unwrap();

        assert_eq!(res, 1);

        let res = db.get_last_index(keychain).unwrap();
        assert!(res.is_none());
    }

    pub fn test_check_descriptor_checksum<D: Database>(mut db: D) {
        // insert checksum associated to keychain
        let checksum = "1cead456".as_bytes();
        let keychain = KeychainKind::External;
        let _res = db.check_descriptor_checksum(keychain, checksum);

        // check if `check_descriptor_checksum` throws
        // `Error::ChecksumMismatch` error if the
        // function is passed a checksum that does
        // not match the one initially inserted
        let checksum = "1cead454".as_bytes();
        let keychain = KeychainKind::External;
        let res = db.check_descriptor_checksum(keychain, checksum);

        assert!(res.is_err());
    }

    // TODO: more tests...
}
