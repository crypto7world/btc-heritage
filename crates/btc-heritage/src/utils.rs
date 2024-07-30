use std::{fmt::Write, str::FromStr, sync::OnceLock};

use crate::{
    bitcoin::{
        psbt::PartiallySignedTransaction, secp256k1::Secp256k1, Address, Network, Transaction,
    },
    errors::Error,
    miniscript::psbt::PsbtExt,
};
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
        #[cfg(not(any(test, feature = "database-tests")))]
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
            #[cfg(not(any(test, feature = "database-tests")))]
            Err(_) => {
                log::warn!(
                    "environment variable `BITCOIN_NETWORK` is not set. Using Network::Bitcoin."
                );
                Network::Bitcoin
            }
        };
        #[cfg(any(test, feature = "database-tests"))]
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

/// Module allowing Ser/De for the [Amount] struct
pub(crate) mod amount_serde {
    use bdk::bitcoin::Amount;

    pub fn serialize<S>(amount: &Amount, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_u64(amount.to_sat())
    }
    pub fn deserialize<'de, D>(deserializer: D) -> Result<Amount, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct AmountVisitor;
        impl<'de> serde::de::Visitor<'de> for AmountVisitor {
            type Value = Amount;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a positive integer representing a satoshi amount")
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(Amount::from_sat(value))
            }
            // Because serde_dynamo just ignores the hint that we expect a u64. Thx dude.
            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                if value >= 0 {
                    Ok(Amount::from_sat(value as u64))
                } else {
                    Err(serde::de::Error::invalid_type(
                        serde::de::Unexpected::Signed(value),
                        &self,
                    ))
                }
            }
        }
        deserializer.deserialize_u64(AmountVisitor)
    }
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
