use core::{cmp::Ordering, fmt::Write, str::FromStr};
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use crate::{
    bitcoin::{
        psbt::PartiallySignedTransaction, secp256k1::Secp256k1, Address, Network, Transaction,
    },
    errors::Error,
    miniscript::psbt::PsbtExt,
};

use bdk::bitcoin::Txid;
use serde_json::json;

/// The average time, in second, to produce a block
/// The Bitcoin network targets 10 minutes
pub const AVERAGE_BLOCK_TIME_SEC: u32 = 60 * 10;

pub fn bytes_to_hex_string<B: AsRef<[u8]>>(bytes: B) -> String {
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        write!(s, "{:02x}", byte).expect("pulling hex repr of a byte should never fail");
    }
    s
}

pub fn bitcoin_network_from_env() -> &'static Network {
    static BITCOIN_NETWORK: OnceLock<Network> = OnceLock::new();
    BITCOIN_NETWORK.get_or_init(|| {
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
        bitcoin_network
    })
}

pub fn string_to_address(s: &str) -> Result<Address, Error> {
    Ok(Address::from_str(s)
        .map_err(|e| {
            log::error!("Could not parse {s}: {e:#}");
            Error::InvalidAddressString(s.to_owned(), *bitcoin_network_from_env())
        })?
        .require_network(*bitcoin_network_from_env())
        .map_err(|_| Error::InvalidAddressString(s.to_owned(), *bitcoin_network_from_env()))?)
}

/// Returns the current timestamp, as the number of seconds since UNIX_EPOCH
pub fn timestamp_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

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
/// Sort a [Vec] of Transaction-like objects that have
/// parents information using the provided functions that
/// for any T can extract a [BlockHeight], a [Txid] and a HashSet of parents
///
/// `extract_txid_and_bh` is separate from `extract_parents` because we expect
/// that `extract_txid_and_bh` is virtually free whereas `extract_parents` could be more costly
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
}
