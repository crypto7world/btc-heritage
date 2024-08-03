use std::io::{stdin, stdout, Write};

use btc_heritage_wallet::errors::{Error, Result};

pub fn ask_user_confirmation(prompt: &str) -> Result<bool> {
    print!("{prompt} Answer yes or no (default no): ");
    stdout().flush().map_err(|e| {
        log::error!("Could not display the confirmation prompt: {e}");
        Error::Generic(e.to_string())
    })?;

    let mut s = String::new();
    stdin().read_line(&mut s).map_err(|e| {
        log::error!("Not a correct string: {e}");
        Error::Generic(e.to_string())
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
    let passphrase1 = rpassword::prompt_password("Please enter your password: ")
        .map_err(|e| Error::Generic(e.to_string()))?;
    if double_check {
        let passphrase2 = rpassword::prompt_password("Please re-enter your password: ")
            .map_err(|e| Error::Generic(e.to_string()))?;
        if passphrase1 != passphrase2 {
            return Err(Error::Generic("Passwords did not match".to_owned()));
        }
    }
    Ok(passphrase1)
}
