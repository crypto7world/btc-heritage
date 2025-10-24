use core::{cmp::Ordering, fmt::Write, str::FromStr};
use std::collections::{HashMap, HashSet};

use crate::{
    bitcoin::{psbt::PartiallySignedTransaction, secp256k1::Secp256k1, Address, Transaction, Txid},
    errors::Error,
    miniscript::psbt::PsbtExt,
};

use serde_json::json;

/// The average time, in second, to produce a block
/// The Bitcoin network targets 10 minutes
pub const AVERAGE_BLOCK_TIME_SEC: u32 = 60 * 10;

/// Converts bytes to a lowercase hexadecimal string representation
///
/// # Examples
///
/// ```
/// # use btc_heritage::utils::bytes_to_hex_string;
/// let bytes = [0xde, 0xad, 0xbe, 0xef];
/// assert_eq!(bytes_to_hex_string(&bytes), "deadbeef");
/// ```
pub fn bytes_to_hex_string<B: AsRef<[u8]>>(bytes: B) -> String {
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        write!(s, "{:02x}", byte).expect("writing in a String should never fails");
    }
    s
}

/// Bitcoin network configuration module
///
/// This module provides thread-safe access to the current Bitcoin network configuration.
/// The network can be set once and then accessed from anywhere in the application.
/// If no network is explicitly set, it will be initialized based on the `BITCOIN_NETWORK` environment variable.
pub mod bitcoin_network {
    use crate::bitcoin::{network::Magic, Network};
    use core::sync::atomic::{AtomicU32, Ordering};

    /// Global atomic storage for the Bitcoin network magic bytes
    static BITCOIN_NETWORK: AtomicU32 = AtomicU32::new(0); //Uninitialized

    /// Gets the current Bitcoin network
    ///
    /// Returns the currently configured Bitcoin network. If no network has been
    /// explicitly set, this will initialize the network based on the
    /// `BITCOIN_NETWORK`environment variable.
    ///
    /// # Examples
    ///
    /// ```
    /// # use btc_heritage::utils::bitcoin_network;
    /// let network = bitcoin_network::get();
    /// println!("Current network: {:?}", network);
    /// ```
    pub fn get() -> Network {
        match Network::from_magic(magic_from_atomic()) {
            Some(network) => network,
            None => init(),
        }
    }

    /// Sets the Bitcoin network globally
    ///
    /// This function stores the network configuration globally so it can be
    /// accessed from anywhere in the application via [`get()`].
    ///
    /// # Examples
    ///
    /// ```
    /// # use btc_heritage::utils::bitcoin_network;
    /// # use btc_heritage::bitcoin;
    /// use bitcoin::Network;
    /// bitcoin_network::set(Network::Testnet);
    /// ```
    pub fn set(network: Network) {
        let magic_u8 = network.magic().to_bytes();
        // SAFETY: u32 and [u8; 4] have the same size and alignment
        BITCOIN_NETWORK.store(unsafe { core::mem::transmute(magic_u8) }, Ordering::Relaxed);
    }

    /// Initializes the Bitcoin network from environment or feature flags
    fn init() -> Network {
        #[cfg(not(any(test, feature = "database-tests", feature = "psbt-tests")))]
        let bitcoin_network = match std::env::var("BITCOIN_NETWORK") {
            Ok(bitcoin_network) => match bitcoin_network.as_str() {
                "bitcoin" => Network::Bitcoin,
                "testnet" => Network::Testnet,
                "regtest" => Network::Regtest,
                "signet" => Network::Signet,
                _ => {
                    log::warn!(
                        "environment variable `BITCOIN_NETWORK` is set to unknown value: \
                        \"{bitcoin_network}\". Using Network::Testnet."
                    );
                    Network::Testnet
                }
            },
            Err(_) => {
                log::warn!(
                    "environment variable `BITCOIN_NETWORK` is not set. Using Network::Bitcoin."
                );
                Network::Bitcoin
            }
        };
        #[cfg(any(test, feature = "database-tests", feature = "psbt-tests"))]
        let bitcoin_network = Network::Regtest;

        log::info!("BITCOIN_NETWORK={bitcoin_network:?}");
        set(bitcoin_network);
        bitcoin_network
    }

    /// Converts the atomic u32 value back to Magic bytes
    fn magic_from_atomic() -> Magic {
        let magic_u32 = BITCOIN_NETWORK.load(Ordering::Relaxed);
        // SAFETY: u32 and [u8; 4] have the same size and alignment
        Magic::from_bytes(unsafe { core::mem::transmute(magic_u32) })
    }
}

/// Parses a string into a Bitcoin address for the current network
///
/// This function validates that the address string is properly formatted and
/// matches the currently configured Bitcoin network.
///
/// # Errors
///
/// Returns [`Error::InvalidAddressString`] if the string cannot be parsed as
/// a valid Bitcoin address or if the address is for a different network than
/// the currently configured one.
///
/// # Examples
///
/// ```
/// # use btc_heritage::utils::{string_to_address, bitcoin_network};
/// # use btc_heritage::bitcoin::Network;
/// // Set the Network to testnet
/// bitcoin_network::set(Network::Testnet);
/// // Testnet address (tb1) is ok
/// assert!(string_to_address("tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").is_ok());
/// // Mainnet address (bc1) is not
/// assert!(string_to_address("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx").is_err());
/// ```
pub fn string_to_address(s: &str) -> Result<Address, Error> {
    let network = bitcoin_network::get();
    Ok(Address::from_str(s)
        .map_err(|e| {
            log::error!("Could not parse {s}: {e:#}");
            Error::InvalidAddressString(s.to_owned(), network)
        })?
        .require_network(network)
        .map_err(|_| Error::InvalidAddressString(s.to_owned(), network))?)
}

#[cfg(test)]
pub mod testtime {
    use std::cell::RefCell;

    thread_local! {
        static FAKE_SYSTEM_TIMESTAMP: RefCell<Option<u64>> =RefCell::new(None);
    }
    /// Sets a fake timestamp for testing purposes
    ///
    /// When `Some(timestamp)` is provided, subsequent calls to [`timestamp_now()`]
    /// will return this fixed value. When `None` is provided, the real system
    /// time will be used.
    pub fn set_timestamp_now(ts: Option<u64>) {
        FAKE_SYSTEM_TIMESTAMP.with_borrow_mut(|fts| *fts = ts);
    }
    /// Returns the current timestamp, or the fake timestamp if one was set
    pub(super) fn timestamp_now() -> u64 {
        FAKE_SYSTEM_TIMESTAMP.with_borrow(|fts| match *fts {
            Some(ts) => ts,
            None => std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        })
    }
}
/// Returns the current timestamp, as the number of seconds since UNIX_EPOCH
///
/// In test builds, this may return a fake timestamp set by [`testtime::set_timestamp_now()`].
#[cfg(test)]
pub fn timestamp_now() -> u64 {
    testtime::timestamp_now()
}
/// Returns the current timestamp, as the number of seconds since UNIX_EPOCH
#[cfg(not(test))]
pub fn timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Extracts a signed transaction from a finalized PSBT
///
/// This function finalizes the PSBT and extracts the complete signed transaction.
/// All inputs must be properly signed and finalized for this to succeed.
///
/// # Errors
///
/// Returns [`Error::UnfinalizablePsbt`] if:
/// - The PSBT cannot be finalized (missing signatures, invalid signatures, etc.)
/// - The PSBT has malformed structure (mismatched input counts)
/// - Not all inputs are fully signed
pub fn extract_tx(psbt: PartiallySignedTransaction) -> Result<Transaction, Error> {
    log::debug!("extract_tx - psbt: {}", json!(psbt));
    let psbt = psbt.finalize(&Secp256k1::new()).map_err(|(psbt, errors)| {
        log::debug!("finalize psbt error. psbt: {}", json!(psbt));
        for e in errors {
            log::error!("finalize psbt error: {e:#}");
        }
        Error::UnfinalizablePsbt(psbt)
    })?;
    log::debug!("extract_tx - final psbt: {}", json!(psbt));

    let tx_inputs_len = psbt.unsigned_tx.input.len();
    let psbt_inputs_len = psbt.inputs.len();
    if tx_inputs_len != psbt_inputs_len {
        log::error!(
            "Malformed PSBT, {} unsigned tx inputs and {} psbt inputs.",
            tx_inputs_len,
            psbt_inputs_len
        );
        return Err(Error::UnfinalizablePsbt(psbt));
    }
    let signed_tx_inputs_len = psbt.inputs.iter().fold(0, |count, input| {
        if input.final_script_sig.is_some() || input.final_script_witness.is_some() {
            count + 1
        } else {
            count
        }
    });
    if tx_inputs_len != signed_tx_inputs_len {
        log::error!("The PSBT is not finalized, inputs are not fully signed.");
        return Err(Error::UnfinalizablePsbt(psbt));
    }

    let raw_tx = psbt.extract_tx();
    log::debug!("extract_tx - raw_tx: {}", json!(raw_tx));
    Ok(raw_tx)
}

type BlockHeight = Option<u32>;

/// Sorts a vector of transaction-like objects with dependency ordering
///
/// This function sorts transactions so they are in chronological order (oldest to newest)
/// with proper dependency ordering. Transactions in older blocks come first, and within
/// the same block, transactions are ordered so that dependencies come before dependents.
///
/// The sorting algorithm considers:
/// 1. Block height (confirmed transactions before unconfirmed)
/// 2. Transaction dependencies (parents before children)
/// 3. Out-of-block dependencies (transactions with external dependencies first)
/// 4. Transaction ID as final tiebreaker
///
/// # Arguments
///
/// * `v` - Mutable reference to the vector to sort in-place
/// * `extract_txid_and_bh` - Function to extract [Txid] and [BlockHeight]
/// * `extract_parents` - Function to extract parent [Txid]s
///
/// The `extract_txid_and_bh` function is separate from `extract_parents` because
/// extracting the txid and block height is expected to be virtually free, whereas
/// `extract_parents` could be more computationally expensive.
pub fn sort_transactions_with_parents<T, F, P>(
    v: &mut Vec<T>,
    extract_txid_and_bh: F,
    extract_parents: P,
) where
    F: Fn(&T) -> (Txid, BlockHeight),
    P: Fn(&T) -> HashSet<Txid>,
{
    // We must sort the transactions so they are in the correct order, from oldest to newest
    // Trivialy, TXs in older blocks are older. For TXs in the same block, we must order them so
    // that if any TX "A" uses an UTXO of another TX "B", "A" comes after "B".
    let mut tx_index = HashMap::new();
    // First pass, each TX simply stores its block_height and the TX ids it depends on
    for tx in v.iter() {
        let (tx_id, bh) = extract_txid_and_bh(tx);
        let parents_txid_set = extract_parents(tx);
        tx_index.insert(tx_id, (bh, parents_txid_set));
    }
    // Now sort
    v.sort_by(|a, b| {
        let (a_tx_id, a_bh) = extract_txid_and_bh(a);
        let (b_tx_id, b_bh) = extract_txid_and_bh(b);
        // a < b if BlockHeight(a) < BlockHeight(b)
        // for BlockHeight Some < None (None means the TX has not been included in the
        // blockchain yet so it comes after a TX already included)
        (match (a_bh, b_bh) {
            (Some(a_bh), Some(b_bh)) => a_bh.cmp(&b_bh),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        })
        // Then consider the dependencies
        // a < b if a appears in the dependencies of b (recursively)
        // b < a if b appears in the dependencies of a (recursively)
        .then_with(|| {
            // We will only consider the dependencies that are in the same block
            // We are here because a and b are in the same block, so there is no point
            // constructing a dependency tree further than the common block of a and b
            // as our goal is to see iff either one appears in the dependencies of the other
            let block_height_of_interest = a_bh;
            let x_depends_on_y = |x_tx_id, y_tx_id| {
                let mut stack_deps_x = vec![];
                // Initialy, add the direct dependencies of x in the stack
                stack_deps_x.extend(
                    tx_index
                        .get(&x_tx_id)
                        .expect("txid of x is in the HashMap")
                        .1
                        .iter()
                        .cloned(),
                );
                // Now pop through all the elements of the stack
                while let Some(deps_id) = stack_deps_x.pop() {
                    // If we find y, then x depends on y
                    if deps_id == y_tx_id {
                        return true;
                    }
                    // If the dependency is a transaction we have in our index
                    if let Some((bh, deps)) = tx_index.get(&deps_id) {
                        // And the block height of this transaction is our block_height_of_interest
                        if block_height_of_interest == *bh {
                            // Then add the dependencies of this transaction to the stack of dependencies of x
                            stack_deps_x.extend(deps)
                        }
                    }
                }
                false
            };
            // If a depends on b, then a is greater (it comes after)
            if x_depends_on_y(a_tx_id, b_tx_id) {
                Ordering::Greater
            }
            // Else if b depends on a, the a is less (it comes before)
            else if x_depends_on_y(b_tx_id, a_tx_id) {
                Ordering::Less
            }
            // Else equal
            else {
                // At this point, we found no direct dependency between a and b
                // Note than it does not mean there is none. But if there is a dependency
                // it is through transactions that we do not know of (outside of our wallet)
                Ordering::Equal
            }
        })
        // If TXs are in the same block and have no direct depedencies between them
        // We consider that the TX that have depencies outside of the block
        // are likely to comes before
        .then_with(|| {
            let block_height_of_interest = a_bh;
            let has_out_of_block_dependencies = |tx_id| {
                tx_index
                    .get(tx_id)
                    .expect("txid of a is in the HashMap")
                    .1
                    .iter()
                    .any(|dep_id| {
                        tx_index
                            .get(dep_id)
                            .is_some_and(|(bh, _)| *bh != block_height_of_interest)
                    })
            };
            let a_has_out_of_block_dependencies = has_out_of_block_dependencies(&a_tx_id);
            let b_has_out_of_block_dependencies = has_out_of_block_dependencies(&b_tx_id);
            match (
                a_has_out_of_block_dependencies,
                b_has_out_of_block_dependencies,
            ) {
                (true, false) => Ordering::Less,
                (false, true) => Ordering::Greater,
                _ => Ordering::Equal,
            }
        })
        .then_with(|| a_tx_id.cmp(&b_tx_id))
    });
}

#[cfg(test)]
mod tests {

    use crate::tests::{get_test_signed_psbt, get_test_unsigned_psbt, TestPsbt};

    use super::*;

    #[test]
    fn bytes_to_hex_string() {
        let bytes: &[u8] = &[
            0u8, 1u8, 2u8, 3u8, 4u8, 5u8, 6u8, 7u8, 8u8, 9u8, 10u8, 11u8, 12u8, 13u8, 14u8, 15u8,
            16u8, 17u8, 255u8,
        ];
        assert_eq!(
            &super::bytes_to_hex_string(bytes),
            "000102030405060708090a0b0c0d0e0f1011ff"
        );
    }

    // Make sure sur PSBT encoder/decoder is working as expected
    // For a valid PSBT, we should be able to decode and re-encode to fall back on the same initial string
    #[test]
    fn psbt_decode_encode() {
        let psbt = "cHNidP8BAH0BAAAAAcaB48e7y2VbIMLS6Yzx5Z3JUcxaXwBcFRE/nURtzt3yAAAAAAD+////AugDAAAAAAAAFgAUBTcYDfDSjHWzO4fLKmjVEt4mrFr6HQAAAAAAACJRICchM4h1J7JjLAF+h1R217ztsnzmSuwR//HAV8gzNMGiMqkmAAABASuFIgAAAAAAACJRIJtNiZBebBFRj78UlqpUkT9Rd+jrPKOBWF/BPrw5bCQCIhXAavT/+hsR7JA6/BtVihyUkONUcEd3JeABo1TD/cWDem4uIEI90EDRPmmkjWAJlq5gU9pfBS4dsIWQHqyg/QV9RysBrQKgMrJpBEDAimWxwCEWQj3QQNE+aaSNYAmWrmBT2l8FLh2whZAerKD9BX1HKwE5AW7HBCrqauSJY+xub80rLxpnTHoLSwglCgA+5yUEcK4mc8XaClYAAIABAACAcmll6AAAAAAAAAAAIRZq9P/6GxHskDr8G1WKHJSQ41RwR3cl4AGjVMP9xYN6bhkAc8XaClYAAIABAACAAAAAgAEAAAAKAAAAARcgavT/+hsR7JA6/BtVihyUkONUcEd3JeABo1TD/cWDem4BGCBuxwQq6mrkiWPsbm/NKy8aZ0x6C0sIJQoAPuclBHCuJgAAAQUgYi+I/4VbEFhAzNw/lEMWrZ46UAGY+mF/L6GPtoZn2sYBBjAAwC0gQj3QQNE+aaSNYAmWrmBT2l8FLh2whZAerKD9BX1HKwGtAqAysmkEQMCKZbEhB0I90EDRPmmkjWAJlq5gU9pfBS4dsIWQHqyg/QV9RysBOQFuxwQq6mrkiWPsbm/NKy8aZ0x6C0sIJQoAPuclBHCuJnPF2gpWAACAAQAAgHJpZegAAAAAAAAAACEHYi+I/4VbEFhAzNw/lEMWrZ46UAGY+mF/L6GPtoZn2sYZAHPF2gpWAACAAQAAgAAAAIABAAAACwAAAAA=";
        assert_eq!(
            &PartiallySignedTransaction::from_str(psbt)
                .unwrap()
                .to_string(),
            psbt
        );
    }

    // Invalid PSBT
    #[test]
    fn psbt_decode_invalid_string() {
        let psbt = "cHNidP8BAH0BAAAAAcaB48e7y2VbIMLS6YzxUcxaXwBcFRE/nURtzt3yAAAAAAD+////AugDAAAAAAAAFgAUBTcYDfDSjHWzO4fLKmjVEt4mrFr6HQAAAAAAACJRICchM4h1J7JjLAF+h1R217ztsnzmSuwR//HAV8gzNMGiMqkmAAABASuFIgAAAAAAACJRIJtNiZBebBFRj78UlqpUkT9Rd+jrPKOBWF/BPrw5bCQCIhXAavT/+hsR7JA6/BtVihyUkONUcEd3JeABo1TD/cWDem4uIEI90EDRPmmkjWAJlq5gU9pfBS4dsIWQHqyg/QV9RysBrQKgMrJpBEDAimWxwCEWQj3QQNE+aaSNYAmWrmBT2l8FLh2whZAerKD9BX1HKwE5AW7HBCrqauSJY+xub80rLxpnTHoLSwglCgA+5yUEcK4mc8XaClYAAIABAACAcmll6AAAAAAAAAAAIRZq9P/6GxHskDr8G1WKHJSQ41RwR3cl4AGjVMP9xYN6bhkAc8XaClYAAIABAACAAAAAgAEAAAAKAAAAARcgavT/+hsR7JA6/BtVihyUkONUcEd3JeABo1TD/cWDem4BGCBuxwQq6mrkiWPsbm/NKy8aZ0x6C0sIJQoAPuclBHCuJgAAAQUgYi+I/4VbEFhAzNw/lEMWrZ46UAGY+mF/L6GPtoZn2sYBBjAAwC0gQj3QQNE+aaSNYAmWrmBT2l8FLh2whZAerKD9BX1HKwGtAqAysmkEQMCKZbEhB0I90EDRPmmkjWAJlq5gU9pfBS4dsIWQHqyg/QV9RysBOQFuxwQq6mrkiWPsbm/NKy8aZ0x6C0sIJQoAPuclBHCuJnPF2gpWAACAAQAAgHJpZegAAAAAAAAAACEHYi+I/4VbEFhAzNw/lEMWrZ46UAGY+mF/L6GPtoZn2sYZAHPF2gpWAACAAQAAgAAAAIABAAAACwAAAAA=";
        assert!(PartiallySignedTransaction::from_str(psbt).is_err());
    }

    #[test]
    fn extract_tx_only_succeed_on_signed_psbt() {
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::OwnerDrain)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::OwnerRecipients)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::BackupFuture)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::WifeFuture)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::BrotherFuture)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::BackupPresent)).is_ok());
        assert!(extract_tx(get_test_signed_psbt(TestPsbt::WifePresent)).is_ok());

        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::OwnerDrain)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::OwnerRecipients)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::BackupFuture)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::WifeFuture)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::BrotherFuture)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::BackupPresent)).is_err());
        assert!(extract_tx(get_test_unsigned_psbt(TestPsbt::WifePresent)).is_err());
    }

    mod sort_transactions_tests {
        use super::*;
        use std::collections::HashSet;

        /// Mock transaction type for testing
        #[derive(Debug, Clone, PartialEq)]
        struct MockTx {
            txid: Txid,
            block_height: Option<u32>,
            parents: HashSet<Txid>,
        }

        impl MockTx {
            fn new(txid_str: &str, block_height: Option<u32>, parents: Vec<&str>) -> Self {
                let txid = Txid::from_str(txid_str).unwrap();
                let parents = parents
                    .into_iter()
                    .map(|p| Txid::from_str(p).unwrap())
                    .collect();
                Self {
                    txid,
                    block_height,
                    parents,
                }
            }

            fn with_no_parents(txid_str: &str, block_height: Option<u32>) -> Self {
                Self::new(txid_str, block_height, vec![])
            }
        }

        // Helper txids for testing
        const TX_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const TX_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const TX_C: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
        const TX_D: &str = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        const TX_E: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
        const TX_F: &str = "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";

        fn extract_txid_and_bh(tx: &MockTx) -> (Txid, Option<u32>) {
            (tx.txid, tx.block_height)
        }

        fn extract_parents(tx: &MockTx) -> HashSet<Txid> {
            tx.parents.clone()
        }

        #[test]
        fn test_sort_by_block_height_confirmed_first() {
            let mut txs = vec![
                MockTx::with_no_parents(TX_A, None),      // Unconfirmed
                MockTx::with_no_parents(TX_C, Some(50)),  // Block 50
                MockTx::with_no_parents(TX_B, Some(150)), // Block 150
                MockTx::with_no_parents(TX_D, Some(100)), // Block 100
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Should be sorted by block height: 50, 100, 150, None (unconfirmed last)
            assert_eq!(txs[0].block_height, Some(50));
            assert_eq!(txs[1].block_height, Some(100));
            assert_eq!(txs[2].block_height, Some(150));
            assert_eq!(txs[3].block_height, None);
        }

        #[test]
        fn test_sort_by_dependencies_within_same_block() {
            let mut txs = vec![
                MockTx::new(TX_B, Some(100), vec![TX_A]), // B depends on A
                MockTx::with_no_parents(TX_A, Some(100)), // A has no dependencies
                MockTx::new(TX_C, Some(100), vec![TX_B]), // C depends on B
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Should be sorted by dependency: A -> B -> C
            assert_eq!(txs[0].txid, Txid::from_str(TX_A).unwrap());
            assert_eq!(txs[1].txid, Txid::from_str(TX_B).unwrap());
            assert_eq!(txs[2].txid, Txid::from_str(TX_C).unwrap());
        }

        #[test]
        fn test_sort_complex_dependencies_same_block() {
            let mut txs = vec![
                MockTx::new(TX_A, Some(100), vec![TX_B, TX_C]), // A depends on B and C
                MockTx::new(TX_B, Some(100), vec![TX_D]),       // B depends on D
                MockTx::new(TX_C, Some(100), vec![TX_D]),       // C depends on D
                MockTx::with_no_parents(TX_D, Some(100)),       // D has no dependencies
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // D should come first, then B and C (because txid_b < txid_c),
            // then A which depends on both B and C
            assert_eq!(txs[0].txid, Txid::from_str(TX_D).unwrap());
            assert_eq!(txs[1].txid, Txid::from_str(TX_B).unwrap());
            assert_eq!(txs[2].txid, Txid::from_str(TX_C).unwrap());
            assert_eq!(txs[3].txid, Txid::from_str(TX_A).unwrap());
        }

        #[test]
        fn test_sort_mixed_block_heights_with_dependencies() {
            let mut txs = vec![
                MockTx::new(TX_C, Some(200), vec![TX_B]), // C in block 200, depends on B
                MockTx::with_no_parents(TX_A, Some(100)), // A in block 100
                MockTx::new(TX_B, Some(150), vec![TX_A]), // B in block 150, depends on A
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Should be sorted by block height regardless of dependencies across blocks
            assert_eq!(txs[0].block_height, Some(100)); // TX_A
            assert_eq!(txs[1].block_height, Some(150)); // TX_B
            assert_eq!(txs[2].block_height, Some(200)); // TX_C
        }

        #[test]
        fn test_sort_unconfirmed_with_dependencies() {
            let mut txs = vec![
                MockTx::new(TX_B, None, vec![TX_C]), // B unconfirmed, depends on C
                MockTx::with_no_parents(TX_C, None), // C unconfirmed, no deps
                MockTx::new(TX_A, None, vec![TX_B]), // A unconfirmed, depends on B
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Should be sorted by dependency even when unconfirmed: C -> B -> A
            assert_eq!(txs[0].txid, Txid::from_str(TX_C).unwrap());
            assert_eq!(txs[1].txid, Txid::from_str(TX_B).unwrap());
            assert_eq!(txs[2].txid, Txid::from_str(TX_A).unwrap());
        }

        #[test]
        fn test_sort_out_of_block_dependencies_priority() {
            let mut txs = vec![
                MockTx::with_no_parents(TX_A, Some(100)), // A has no dependencies
                MockTx::new(TX_B, Some(100), vec![TX_C]), // B depends on TX_C
                MockTx::with_no_parents(TX_C, Some(50)),  // C is out of block 100
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            assert!(txs[0].block_height == Some(50));
            assert!(txs[1].block_height == Some(100));
            assert!(txs[2].block_height == Some(100));

            // Within block 100: B (external deps) should come first, then A, then C
            let block100_txs = &txs[1..3];
            // B should come first because it has out-of-block dependencies
            assert_eq!(block100_txs[0].txid, Txid::from_str(TX_B).unwrap()); // External deps first
            assert_eq!(block100_txs[1].txid, Txid::from_str(TX_A).unwrap()); // Then A
        }

        #[test]
        fn test_sort_txid_as_final_tiebreaker() {
            let mut txs = vec![
                MockTx::with_no_parents(TX_B, Some(100)), // B comes after A lexicographically
                MockTx::with_no_parents(TX_A, Some(100)), // A comes before F lexicographically
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Should be sorted by txid as final tiebreaker: A before B
            assert_eq!(txs[0].txid, Txid::from_str(TX_A).unwrap());
            assert_eq!(txs[1].txid, Txid::from_str(TX_B).unwrap());
        }

        #[test]
        fn test_sort_empty_vector() {
            let mut txs: Vec<MockTx> = vec![];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            assert!(txs.is_empty());
        }

        #[test]
        fn test_sort_single_transaction() {
            let mut txs = vec![MockTx::with_no_parents(TX_A, Some(100))];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            assert_eq!(txs.len(), 1);
            assert_eq!(txs[0].txid, Txid::from_str(TX_A).unwrap());
        }

        #[test]
        fn test_sort_cross_block_and_intra_block_dependencies() {
            let mut txs = vec![
                MockTx::new(TX_E, Some(200), vec![TX_D]), // E in block 200, depends on D
                MockTx::new(TX_D, Some(200), vec![TX_C]), // D in block 200, depends on C (different block)
                MockTx::new(TX_C, Some(100), vec![TX_A]), // C in block 100, depends on A
                MockTx::new(TX_B, Some(100), vec![TX_F]), // B in block 100, external dep
                MockTx::with_no_parents(TX_A, Some(100)), // A in block 100, no deps
                MockTx::with_no_parents(TX_F, Some(50)),  // F is out of block 100
            ];

            sort_transactions_with_parents(&mut txs, extract_txid_and_bh, extract_parents);

            // Block 50 should come before block 100 which comes before block 200
            assert!(txs[0].block_height == Some(50));
            assert!(txs[1].block_height == Some(100));
            assert!(txs[2].block_height == Some(100));
            assert!(txs[3].block_height == Some(100));
            assert!(txs[4].block_height == Some(200));
            assert!(txs[5].block_height == Some(200));

            // Within block 100: B (external deps) should come first, then A, then C
            let block100_txs = &txs[1..4];
            assert_eq!(block100_txs[0].txid, Txid::from_str(TX_B).unwrap()); // External deps first
            assert_eq!(block100_txs[1].txid, Txid::from_str(TX_A).unwrap()); // Then A
            assert_eq!(block100_txs[2].txid, Txid::from_str(TX_C).unwrap()); // Then C (depends on A)

            // Within block 200: D then E (E depends on D)
            let block200_txs = &txs[4..6];
            assert_eq!(block200_txs[0].txid, Txid::from_str(TX_D).unwrap());
            assert_eq!(block200_txs[1].txid, Txid::from_str(TX_E).unwrap());
        }
    }
}
