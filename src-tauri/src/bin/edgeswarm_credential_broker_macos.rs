#[cfg(target_os = "macos")]
mod macos {
use std::io::{Read, Write};
use zeroize::Zeroizing;

const SERVICE: &str =
    "com.edgeswarm.node.restart-credential.v2";

const ACCOUNT: &str =
    "provider";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(
        SERVICE,
        ACCOUNT,
    )
    .map_err(|_| {
        "macos_broker_keychain_entry_failed"
            .to_string()
    })
}

fn store_credential_v2() -> Result<(), String> {
    let mut password =
        Zeroizing::new(String::new());

    std::io::stdin()
        .read_to_string(&mut password)
        .map_err(|_| {
            "macos_broker_stdin_read_failed"
                .to_string()
        })?;

    if password.is_empty() {
        return Err(
            "macos_broker_credential_empty".into()
        );
    }

    entry()?
        .set_password(password.as_str())
        .map_err(|_| {
            "macos_broker_keychain_write_failed"
                .to_string()
        })?;

    println!(
        "MACOS_CREDENTIAL_BROKER_STORED=true"
    );

    Ok(())
}

fn emit_credential_v2() -> Result<(), String> {
    let password =
        entry()?
            .get_password()
            .map_err(|_| {
                "macos_broker_keychain_read_failed"
                    .to_string()
            })?;

    let password =
        Zeroizing::new(password);

    if password.is_empty() {
        return Err(
            "macos_broker_credential_empty".into()
        );
    }

    // stdout is exclusively the private credential pipe.
    let mut stdout =
        std::io::stdout().lock();

    stdout
        .write_all(
            password.as_bytes()
        )
        .map_err(|_| {
            "macos_broker_stdout_write_failed"
                .to_string()
        })?;

    stdout
        .flush()
        .map_err(|_| {
            "macos_broker_stdout_flush_failed"
                .to_string()
        })?;

    Ok(())
}

fn run() -> Result<(), String> {
    let mode =
        std::env::args()
            .nth(1)
            .ok_or_else(|| {
                "macos_broker_mode_missing"
                    .to_string()
            })?;

    match mode.as_str() {
        "--store-restart-credential" =>
            store_credential_v2(),

        "--emit-restart-credential" =>
            emit_credential_v2(),

        _ =>
            Err(
                "macos_broker_mode_invalid".into()
            ),
    }
}

pub fn entrypoint() {
    if let Err(error) = run() {
        eprintln!(
            "MACOS_CREDENTIAL_BROKER_ERROR={}",
            error.replace('\n', " ")
        );

        std::process::exit(1);
    }
}
}

#[cfg(target_os = "macos")]
fn main() {
    macos::entrypoint();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!(
        "MACOS_CREDENTIAL_BROKER_ERROR=macos_only"
    );

    std::process::exit(1);
}
