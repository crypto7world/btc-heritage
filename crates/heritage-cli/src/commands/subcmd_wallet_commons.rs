use std::{path::PathBuf, str::FromStr};

use btc_heritage_wallet::bitcoin::{Address, Amount};
use clap::builder::{PossibleValuesParser, TypedValueParser};

/// Common sub-command for both service and local wallets.
#[derive(Debug, Clone, clap::Subcommand)]
pub enum WalletSubcmd {
    /// Creates a new Heritage wallet from a randomly generated seed or a given one or a hardware-wallet
    Create {
        #[command(flatten)]
        key_source: CreateKeySourceArgsGroup,
        #[arg(
            short = 'p',
            long,
            default_value_t = false,
            requires = "generate",
            requires = "seed",
            conflicts_with = "hardware_device"
        )]
        /// Signal that the seed is password-protected. Invalid if used with '--hardware-device'.
        with_password: bool,
    },
    /// Restore an existing Heritage wallet from a backup file containing its public descriptors and either a seed or hardware-wallet
    Restore {
        #[arg(value_hint = clap::ValueHint::FilePath)]
        /// The path to an Heritage backup file containing the Descriptors of an Heritage wallet.
        backup_file: PathBuf,
        #[command(flatten)]
        ks_args: KeySourceRestoreArgs,
    },
    /// Retrieve the backup file containing the public descriptors of the Heritage wallet
    Backup,
    /// Manage the Heritage configuration of the wallet
    HeritageConfig,
    /// Manage the Account eXtended Public Keys of the wallet
    AccountXpubs,
    /// Sync the wallet from the Bitcoin network
    Sync,
    /// Generate a new Bitcoin address for the Heritage wallet
    GetAddress,
    /// Create a Partially Signed Bitcoin Transaction (PSBT), a.k.a an Unsigned TX, from the provided receipients information
    SendBitcoins {
        #[arg(short, long, value_name="ADDRESS>:<AMOUNT", required = true, value_parser=parse_recipient)]
        /// A recipient for our BTC
        recipient: Vec<(Address, Amount)>,
        #[arg(short, long, default_value_t = false)]
        /// Immediately sign the PSBT
        sign: bool,
        #[arg(short, long, default_value_t = false, requires = "sign")]
        /// Immediately broadcast the PSBT after signing it
        broadcast: bool,
        #[arg(short = 'y', long, default_value_t = false, requires = "broadcast")]
        /// Broadcast without asking for confirmation{n}
        /// /!\ BE VERY CAREFULL with that option /!\.
        skip_confirmation: bool,
    },
    /// Sign every sign-able inputs of the given Partially Signed Bitcoin Transaction (PSBT)
    SignPsbt {
        /// The PSBT to sign
        psbt: String,
        #[arg(long, default_value_t = false)]
        /// Immediately broadcast the PSBT after signing it
        broadcast: bool,
    },
    /// Extract a raw transaction from the given Partially Signed Bitcoin Transaction (PSBT) and broadcast it to the Bitcoin network
    BroadcastPsbt {
        /// The PSBT to broadcast. Must have every inputs correctly signed for this to work.
        psbt: String,
    },
    /// Display infos on the given Partially Signed Bitcoin Transaction (PSBT)
    DisplayPsbt {
        /// The PSBT to display.
        psbt: String,
    },
}

fn parse_recipient(val: &str) -> Result<(Address, Amount), String> {
    static ERR_MSG: &'static str = "invalid recipient. Must be <ADDRESS>:<AMOUNT>";

    if !val.contains(':') {
        return Err(ERR_MSG.to_string());
    }

    let mut parts = val.split(':');
    let addr = parts.next().ok_or_else(|| ERR_MSG.to_string())?;
    let addr = Address::from_str(addr)
        .map_err(|e| e.to_string())?
        .assume_checked();

    let amount = parts.next().ok_or_else(|| ERR_MSG.to_string())?;
    let amount = amount.parse::<Amount>().map_err(|e| e.to_string())?;

    if parts.next().is_some() {
        return Err(ERR_MSG.to_string());
    }

    Ok((addr, amount))
}
#[derive(Debug, Clone, clap::Args)]
#[group(required = true, multiple = false)]
pub struct CreateKeySourceArgsGroup {
    #[arg(long, value_name = "WORD_COUNT", group = "key_source", value_parser=PossibleValuesParser::new(["12", "18", "24"]).map(|s| s.parse::<usize>().unwrap()))]
    /// Generates a new seed mnemonic for the wallet with the given word count (12, 18 or 24).
    generate: Option<usize>,
    #[arg(long, value_name = "WORD", group = "key_source", num_args=2..24)]
    /// The mnemonic phrase to use as a seed for the Heritage wallet.
    seed: Option<Vec<String>>,
    #[arg(long, group = "key_source", default_value_t = false)]
    /// Use the connected Ledger device as the backend for your Heritage wallet
    ledger: bool,
}

/// Common options for Restore and Bind.
#[derive(Debug, Clone, clap::Args)]
pub struct KeySourceRestoreArgs {
    #[command(flatten)]
    /// The mnemonic phrase to use as a seed for the Heritage wallet.
    key_source: KeySourceArgsGroup,
    #[arg(
        short = 'p',
        long,
        default_value_t = false,
        requires = "seed",
        conflicts_with = "ledger"
    )]
    /// Signal that the seed is password-protected. Invalid if used with '--ledger'.
    with_password: bool,
}

#[derive(Debug, Clone, clap::Args)]
#[group(required = true, multiple = false)]
pub struct KeySourceArgsGroup {
    #[arg(long, value_name = "WORD", num_args=2..24)]
    /// The mnemonic phrase to use as a seed for the Heritage wallet.
    seed: Option<Vec<String>>,
    #[arg(long, group = "key_source", default_value_t = false)]
    /// Use the connected Ledger device as the backend for your Heritage wallet
    ledger: bool,
}
