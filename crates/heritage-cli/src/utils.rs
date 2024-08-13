use std::{
    collections::HashMap,
    io::{stdin, stdout, Write},
};

use btc_heritage_wallet::{
    errors::{Error, Result},
    heritage_api_client::Fingerprint,
    BoundFingerprint, Database, DatabaseItem, Heir, HeirWallet, Wallet,
};

pub fn ask_user_confirmation(prompt: &str) -> Result<bool> {
    print!("{prompt} Answer \"yes\" or \"no\" (default \"no\"): ");
    stdout().flush().map_err(|e| {
        log::error!("Could not display the confirmation prompt: {e}");
        Error::generic(e)
    })?;

    let mut s = String::new();
    stdin().read_line(&mut s).map_err(|e| {
        log::error!("Not a correct string: {e}");
        Error::generic(e)
    })?;

    // Remove the final \r\n, if present
    if let Some('\n') = s.chars().next_back() {
        s.pop();
    }
    if let Some('\r') = s.chars().next_back() {
        s.pop();
    }
    Ok(s == "yes".to_owned())
}

pub fn prompt_user_for_password(double_check: bool) -> Result<String> {
    let passphrase1 =
        rpassword::prompt_password("Please enter your password: ").map_err(Error::generic)?;
    if double_check {
        let passphrase2 = rpassword::prompt_password("Please re-enter your password: ")
            .map_err(Error::generic)?;
        if passphrase1 != passphrase2 {
            return Err(Error::Generic("Passwords did not match".to_owned()));
        }
    }
    Ok(passphrase1)
}

pub fn get_fingerprints(db: &Database) -> Result<HashMap<Fingerprint, String>> {
    let mut map = HashMap::new();
    map.extend(Heir::all_in_db(&db)?.iter().filter_map(|h| {
        h.fingerprint()
            .ok()
            .map(|f| (f, format!("heir:{}", h.name())))
    }));
    map.extend(HeirWallet::all_in_db(&db)?.iter().filter_map(|hw| {
        hw.fingerprint()
            .ok()
            .map(|f| (f, format!("heir-wallet:{}", hw.name())))
    }));
    map.extend(Wallet::all_in_db(&db)?.iter().filter_map(|w| {
        w.fingerprint()
            .ok()
            .map(|f| (f, format!("wallet:{}", w.name())))
    }));
    Ok(map)
}
