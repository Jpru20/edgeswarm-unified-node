#![cfg(target_os = "macos")]

use zeroize::Zeroizing;

const SERVICE: &str = "com.edgeswarm.node.restart-credential";
const ACCOUNT: &str = "provider";

fn entry() -> Result<keyring::Entry, String> {

    keyring::Entry::new(SERVICE, ACCOUNT).map_err(|_| "macos_keychain_entry_failed".into())
}

pub fn persist_macos_restart_credential_v1(password: &str) -> Result<(), String> {
    if password.is_empty() {
        return Err("macos_restart_credential_empty".into());
    }

    entry()?
        .set_password(password)
        .map_err(|_| "macos_keychain_write_failed".into())
}

pub fn read_macos_restart_credential_v1() -> Result<Zeroizing<String>, String> {
    let password = entry()?
        .get_password()
        .map_err(|_| "macos_keychain_read_failed".to_string())?;

    if password.is_empty() {
        return Err("macos_restart_credential_empty".into());
    }

    Ok(Zeroizing::new(password))
}
