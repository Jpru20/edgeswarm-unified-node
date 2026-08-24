use edgeswarm_unified_node_lib::core::{
    auth_client::SupabaseAuthClient,
    hardware_identity::HardwareIdentity,
    production_task_http::read_auth,
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault,
};
use std::env;
use zeroize::Zeroizing;

fn run() -> Result<(), String> {
    let wallet_password = Zeroizing::new(
        env::var("EDGESWARM_CI_WALLET_PASSWORD")
            .map_err(|_| "ci_wallet_password_missing".to_string())?,
    );

    if wallet_password.trim().is_empty() {
        return Err("ci_wallet_password_empty".into());
    }

    let auth_client = SupabaseAuthClient::from_env()?;
    let ensured = auth_client.ensure_valid_session(true)?;

    if !ensured.session.mfa_verified() {
        return Err("ci_auth_session_not_mfa_verified".into());
    }

    let auth = read_auth()?;
    let hardware = HardwareIdentity::detect()?;
    let wallet_client = WorkerWalletClient::from_env()?;

    let rows = wallet_client.rows_for_email(&auth.access_token, &auth.provider_email)?;

    let (wallet, action) = match select_wallet_row(&rows, &hardware.hardware_id)? {
        WalletRowDecision::ExactDevice { row_index } => {
            let private_key = Zeroizing::new(wallet_vault::decrypt(
                &rows[row_index].private_key,
                wallet_password.as_str(),
                &auth.provider_email,
            )?);

            (
                DeviceWallet::from_private_key(private_key.as_str())?,
                "reuse_exact_device",
            )
        }

        WalletRowDecision::ClaimLegacy { .. } => {
            return Err("ci_refuses_legacy_wallet_claim".into());
        }

        WalletRowDecision::CreateDevice => {
            let generated = DeviceWallet::generate()?;

            let encrypted = wallet_vault::encrypt(
                generated.private_key(),
                wallet_password.as_str(),
                &auth.provider_email,
            )?;

            let status = wallet_client.insert_device(
                &auth.access_token,
                &auth.provider_email,
                &hardware.hardware_id,
                &encrypted,
            )?;

            if !(200..300).contains(&status) {
                return Err(format!("ci_wallet_insert_http_{status}"));
            }

            let verified_rows =
                wallet_client.rows_for_email(&auth.access_token, &auth.provider_email)?;

            let row_index = match select_wallet_row(&verified_rows, &hardware.hardware_id)? {
                WalletRowDecision::ExactDevice { row_index } => row_index,

                _ => {
                    return Err("ci_wallet_insert_verification_failed".into());
                }
            };

            let private_key = Zeroizing::new(wallet_vault::decrypt(
                &verified_rows[row_index].private_key,
                wallet_password.as_str(),
                &auth.provider_email,
            )?);

            let recovered = DeviceWallet::from_private_key(private_key.as_str())?;

            if !recovered
                .wallet_address()
                .eq_ignore_ascii_case(generated.wallet_address())
            {
                return Err("ci_wallet_roundtrip_mismatch".into());
            }

            (recovered, "create_device")
        }
    };

    let public_wallet = WalletPublicIdentity::save_current(wallet.wallet_address())?;

    if !public_wallet
        .hardware_id
        .eq_ignore_ascii_case(&hardware.hardware_id)
    {
        return Err("ci_public_wallet_hardware_mismatch".into());
    }

    println!("CI_DEVICE_WALLET_BOOTSTRAP=true");
    println!("SESSION_MFA_VERIFIED=true");
    println!("SESSION_REFRESHED={}", ensured.refreshed);
    println!("WALLET_ACTION={action}");
    println!("HARDWARE_ID={}", hardware.hardware_id);
    println!("WALLET_ADDRESS={}", wallet.wallet_address());
    println!("WALLET_ROW_VERIFIED=true");
    println!("PUBLIC_WALLET_WRITTEN=true");
    println!("PRIVATE_KEY_PRINTED=false");
    println!("WALLET_PASSWORD_PRINTED=false");
    println!("HEARTBEAT_SENT=false");
    println!("TASK_POLL_SENT=false");

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!(
            "CI_DEVICE_WALLET_BOOTSTRAP_ERROR={}",
            error.replace('\n', " ")
        );
        std::process::exit(2);
    }
}
