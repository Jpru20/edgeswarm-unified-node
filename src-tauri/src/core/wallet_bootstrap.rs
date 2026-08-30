use crate::core::{
    hardware_identity::HardwareIdentity,
    wallet_account::DeviceWallet,
    wallet_client::WorkerWalletClient,
    wallet_identity::{select_wallet_row, WalletRowDecision},
    wallet_public_identity::WalletPublicIdentity,
    wallet_vault,
};
use zeroize::Zeroizing;

pub fn bootstrap_authenticated_device_wallet_v1(
    email: &str,
    access_token: &str,
    password: &str,
) -> Result<(), String> {
    let hardware = HardwareIdentity::detect()?;
    let wallet_client = WorkerWalletClient::from_env()?;
    let rows = wallet_client.rows_for_email(access_token, email)?;

    match select_wallet_row(&rows, &hardware.hardware_id)? {
        WalletRowDecision::ExactDevice { row_index } => {
            let key = Zeroizing::new(wallet_vault::decrypt(
                &rows[row_index].private_key, password, email,
            )?);
            let wallet = DeviceWallet::from_private_key(key.as_str())?;
            let public = WalletPublicIdentity::save_current(wallet.wallet_address())?;
            if !public.hardware_id.eq_ignore_ascii_case(&hardware.hardware_id) {
                return Err("wallet_public_identity_hardware_mismatch".into());
            }
        }
        WalletRowDecision::ClaimLegacy { .. }
        | WalletRowDecision::CreateDevice => {
            let wallet = DeviceWallet::generate()?;
            let encrypted = wallet_vault::encrypt(
                wallet.private_key(), password, email,
            )?;
            let status = wallet_client.insert_device(
                access_token, email, &hardware.hardware_id, &encrypted,
            )?;
            if !(200..300).contains(&status) {
                return Err(format!("wallet_insert_http_{status}"));
            }

            let rows_after = wallet_client.rows_for_email(access_token, email)?;
            let exact = rows_after.iter().find(|row| {
                row.hardware_id.as_deref().unwrap_or("")
                    .eq_ignore_ascii_case(&hardware.hardware_id)
            }).ok_or_else(|| "wallet_insert_not_visible".to_string())?;

            let recovered_key = Zeroizing::new(wallet_vault::decrypt(
                &exact.private_key, password, email,
            )?);
            let recovered = DeviceWallet::from_private_key(recovered_key.as_str())?;
            if !recovered.wallet_address()
                .eq_ignore_ascii_case(wallet.wallet_address()) {
                return Err("wallet_post_insert_identity_mismatch".into());
            }

            let public = WalletPublicIdentity::save_current(wallet.wallet_address())?;
            if !public.hardware_id.eq_ignore_ascii_case(&hardware.hardware_id) {
                return Err("wallet_public_identity_hardware_mismatch".into());
            }
        }
    }

    Ok(())
}
