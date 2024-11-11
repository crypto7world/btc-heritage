use std::collections::HashMap;

use btc_heritage::{
    bitcoin::{bip32::Fingerprint, Address, Amount, FeeRate, Network},
    heritage_wallet::get_expected_tx_weight,
    PartiallySignedTransaction,
};
use heritage_service_api_client::TransactionSummary;
use serde::Serialize;

use crate::errors::{Error, Result};

pub fn serialize_amount<S>(amount: &Amount, serializer: S) -> core::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    if *amount >= Amount::from_btc(0.1).unwrap() {
        serializer.serialize_str(&format!(
            "{} BTC",
            amount.display_in(btc_heritage::bitcoin::Denomination::Bitcoin)
        ))
    } else if *amount >= Amount::from_sat(10000) {
        serializer.serialize_str(&format!(
            "{} mBTC",
            amount.display_in(btc_heritage::bitcoin::Denomination::MilliBitcoin)
        ))
    } else {
        serializer.serialize_str(&format!(
            "{} sat",
            amount.display_in(btc_heritage::bitcoin::Denomination::Satoshi)
        ))
    }
}
pub fn serialize_option<T, S>(
    opt: &Option<T>,
    serializer: S,
) -> core::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: Serialize,
{
    match opt {
        Some(t) => t.serialize(serializer),
        None => serializer.serialize_str("Unknown"),
    }
}
pub fn serialize_option_amount<S>(
    opt: &Option<Amount>,
    serializer: S,
) -> core::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match opt {
        Some(amount) => serialize_amount(amount, serializer),
        None => serializer.serialize_str("Unknown"),
    }
}
pub fn serialize_fee_rate<S>(
    fee_rate: &FeeRate,
    serializer: S,
) -> core::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let fr = fee_rate.to_sat_per_kwu() as f32 / 250.0;
    serializer.serialize_str(&format!("{} sat/vB", fr))
}

#[derive(Debug, Serialize)]
struct InputSummary {
    previous_output: String,
    address: String,
    #[serde(serialize_with = "serialize_amount")]
    amount: Amount,
    known_owning_fingerprints: Vec<Fingerprint>,
    #[serde(serialize_with = "serialize_option")]
    known_owning_wallets: Option<Vec<String>>,
}
#[derive(Debug, Serialize)]
struct OutputSummary {
    address: String,
    #[serde(serialize_with = "serialize_amount")]
    amount: Amount,
    #[serde(serialize_with = "serialize_option")]
    is_owned: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct PsbtSummary {
    inputs: Vec<InputSummary>,
    outputs: Vec<OutputSummary>,
    #[serde(serialize_with = "serialize_amount")]
    total_spend: Amount,
    #[serde(serialize_with = "serialize_amount")]
    send_out: Amount,
    #[serde(serialize_with = "serialize_option_amount")]
    change: Option<Amount>,
    #[serde(serialize_with = "serialize_amount")]
    fee: Amount,
    #[serde(serialize_with = "serialize_fee_rate")]
    fee_rate: FeeRate,
}

impl TryFrom<(&PartiallySignedTransaction, Network)> for PsbtSummary {
    type Error = Error;

    fn try_from(
        value: (&PartiallySignedTransaction, Network),
    ) -> std::result::Result<Self, Self::Error> {
        let (psbt, network) = value;
        Self::try_from((psbt, None, None, network))
    }
}

impl
    TryFrom<(
        &PartiallySignedTransaction,
        &HashMap<Fingerprint, Vec<String>>,
        Network,
    )> for PsbtSummary
{
    type Error = Error;

    fn try_from(
        value: (
            &PartiallySignedTransaction,
            &HashMap<Fingerprint, Vec<String>>,
            Network,
        ),
    ) -> std::result::Result<Self, Self::Error> {
        let (psbt, wallet_fingerprints, network) = value;
        Self::try_from((psbt, None, Some(wallet_fingerprints), network))
    }
}
impl TryFrom<(&PartiallySignedTransaction, &TransactionSummary, Network)> for PsbtSummary {
    type Error = Error;

    fn try_from(
        value: (&PartiallySignedTransaction, &TransactionSummary, Network),
    ) -> std::result::Result<Self, Self::Error> {
        let (psbt, tx_summary, network) = value;
        Self::try_from((psbt, Some(tx_summary), None, network))
    }
}
impl
    TryFrom<(
        &PartiallySignedTransaction,
        &TransactionSummary,
        &HashMap<Fingerprint, Vec<String>>,
        Network,
    )> for PsbtSummary
{
    type Error = Error;

    fn try_from(
        value: (
            &PartiallySignedTransaction,
            &TransactionSummary,
            &HashMap<Fingerprint, Vec<String>>,
            Network,
        ),
    ) -> std::result::Result<Self, Self::Error> {
        let (psbt, tx_summary, wallet_fingerprints, network) = value;
        Self::try_from((psbt, Some(tx_summary), Some(wallet_fingerprints), network))
    }
}

impl
    TryFrom<(
        &PartiallySignedTransaction,
        Option<&TransactionSummary>,
        Option<&HashMap<Fingerprint, Vec<String>>>,
        Network,
    )> for PsbtSummary
{
    type Error = Error;

    fn try_from(
        value: (
            &PartiallySignedTransaction,
            Option<&TransactionSummary>,
            Option<&HashMap<Fingerprint, Vec<String>>>,
            Network,
        ),
    ) -> Result<Self> {
        let (psbt, tx_summary, wallet_fingerprints, network) = value;

        let inputs = psbt
            .unsigned_tx
            .input
            .iter()
            .zip(psbt.inputs.iter())
            .map(|(tx_in, psbt_in)| {
                let (address, amount) = if let Some(witness) = &psbt_in.witness_utxo {
                    (
                        Address::from_script(&witness.script_pubkey, network)
                            .map_err(Error::generic)?,
                        Amount::from_sat(witness.value),
                    )
                } else if let Some(prev_tx) = &psbt_in.non_witness_utxo {
                    let txout = &prev_tx.output[tx_in.previous_output.vout as usize];
                    (
                        Address::from_script(&txout.script_pubkey, network)
                            .map_err(Error::generic)?,
                        Amount::from_sat(txout.value),
                    )
                } else {
                    unreachable!(
                        "PSBT input should always have either witness or non_witness UTXO"
                    );
                };
                let address = address.to_string();
                let known_owning_fingerprints = psbt_in
                    .tap_key_origins
                    .iter()
                    .map(|(_, (_, (f, _)))| *f)
                    .collect::<Vec<_>>();

                let known_owning_wallets = if let Some(wallet_fingerprints) = wallet_fingerprints {
                    Some(
                        known_owning_fingerprints
                            .iter()
                            .filter_map(|f| wallet_fingerprints.get(f))
                            .flatten()
                            .cloned()
                            .collect(),
                    )
                } else {
                    None
                };

                Ok(InputSummary {
                    previous_output: tx_in.previous_output.to_string(),
                    address,
                    amount,
                    known_owning_fingerprints,
                    known_owning_wallets,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let outputs = psbt
            .unsigned_tx
            .output
            .iter()
            .map(|tx_out| {
                let address =
                    Address::from_script(&tx_out.script_pubkey, network).map_err(Error::generic)?;
                let address = address.to_string();
                let amount = Amount::from_sat(tx_out.value);

                let is_owned = if let Some(tx_summary) = tx_summary {
                    Some(
                        tx_summary
                            .owned_outputs
                            .iter()
                            .any(|oo| oo.address.to_string() == address),
                    )
                } else {
                    None
                };

                Ok(OutputSummary {
                    address,
                    amount,
                    is_owned,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let total_spend = inputs.iter().map(|is| is.amount).sum::<Amount>();
        let mut send_out = Amount::ZERO;
        let mut change = Amount::ZERO;
        for o in outputs.iter() {
            if o.is_owned.is_some_and(|c| c) {
                change += o.amount;
            } else {
                send_out += o.amount;
            }
        }
        let fee = total_spend
            .checked_sub(send_out + change)
            .ok_or(Error::Generic(
                "Invalid PSBT. Fee cannot be negative".to_owned(),
            ))?;
        let fee_rate = if let Some(tx_summary) = tx_summary {
            tx_summary.fee_rate
        } else {
            fee / get_expected_tx_weight(psbt)
        };

        Ok(PsbtSummary {
            inputs,
            outputs,
            total_spend,
            send_out,
            change: if tx_summary.is_some() {
                Some(change)
            } else {
                None
            },
            fee,
            fee_rate,
        })
    }
}

impl core::fmt::Display for PsbtSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string_pretty(self).expect("know structure")
        )
    }
}
