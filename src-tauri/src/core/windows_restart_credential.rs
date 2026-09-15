use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

#[cfg(target_os = "windows")]
use std::{ffi::c_void, ptr::null_mut};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{GetLastError, LocalFree},
    Security::Cryptography::{
        CryptProtectData,
        CryptUnprotectData,
        CRYPT_INTEGER_BLOB,
        CRYPTPROTECT_UI_FORBIDDEN,
    },
};

pub fn windows_restart_credential_path_v1() -> PathBuf {
    crate::adapters::app_data_dir()
        .join("wallet_restart_credential.dpapi")
}

#[cfg(target_os = "windows")]
struct DpapiOutBlobV1(CRYPT_INTEGER_BLOB);

#[cfg(target_os = "windows")]
impl Default for DpapiOutBlobV1 {
    fn default() -> Self {
        Self(CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        })
    }
}

#[cfg(target_os = "windows")]
impl Drop for DpapiOutBlobV1 {
    fn drop(&mut self) {
        if self.0.pbData.is_null() {
            return;
        }

        unsafe {
            std::ptr::write_bytes(
                self.0.pbData,
                0,
                self.0.cbData as usize,
            );

            let _ = LocalFree(
                self.0.pbData.cast::<c_void>()
            );
        }
    }
}

#[cfg(target_os = "windows")]
fn dpapi_error_v1(operation: &str) -> String {
    let code = unsafe { GetLastError() };

    format!(
        "{operation}_failed_win32_{code}"
    )
}

#[cfg(target_os = "windows")]
fn blob_bytes_v1(
    blob: &CRYPT_INTEGER_BLOB,
) -> Vec<u8> {
    unsafe {
        std::slice::from_raw_parts(
            blob.pbData,
            blob.cbData as usize,
        )
    }
    .to_vec()
}

#[cfg(target_os = "windows")]
fn protect_current_user_v1(
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(plaintext.len())
            .map_err(|_| "dpapi_input_too_large".to_string())?,
        pbData: plaintext.as_ptr().cast_mut(),
    };

    let mut output =
        DpapiOutBlobV1::default();

    let ok = unsafe {
        CryptProtectData(
            &mut input,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };

    if ok == 0 {
        return Err(
            dpapi_error_v1("dpapi_protect")
        );
    }

    if output.0.pbData.is_null() {
        return Err(
            "dpapi_protect_null_output".into()
        );
    }

    Ok(blob_bytes_v1(&output.0))
}

#[cfg(target_os = "windows")]
fn unprotect_current_user_v1(
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(ciphertext.len())
            .map_err(|_| "dpapi_input_too_large".to_string())?,
        pbData: ciphertext.as_ptr().cast_mut(),
    };

    let mut output =
        DpapiOutBlobV1::default();

    let ok = unsafe {
        CryptUnprotectData(
            &mut input,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output.0,
        )
    };

    if ok == 0 {
        return Err(
            dpapi_error_v1("dpapi_unprotect")
        );
    }

    if output.0.pbData.is_null() {
        return Err(
            "dpapi_unprotect_null_output".into()
        );
    }

    Ok(blob_bytes_v1(&output.0))
}

#[cfg(target_os = "windows")]
pub fn persist_windows_restart_credential_v1(
    password: &str,
) -> Result<(), String> {
    if password.is_empty() {
        return Err(
            "windows_restart_credential_empty".into()
        );
    }

    let path =
        windows_restart_credential_path_v1();

    let parent = path
        .parent()
        .ok_or_else(|| {
            "windows_restart_credential_parent_missing"
                .to_string()
        })?;

    fs::create_dir_all(parent)
        .map_err(|_| {
            "windows_restart_credential_directory_failed"
                .to_string()
        })?;

    let encrypted =
        protect_current_user_v1(
            password.as_bytes()
        )?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);

    let temporary =
        path.with_extension(format!(
            "tmp-{}-{nonce}",
            std::process::id()
        ));

    fs::write(&temporary, encrypted)
        .map_err(|_| {
            "windows_restart_credential_write_failed"
                .to_string()
        })?;

    if path.exists() {
        fs::remove_file(&path)
            .map_err(|_| {
                "windows_restart_credential_replace_failed"
                    .to_string()
            })?;
    }

    fs::rename(&temporary, &path)
        .map_err(|_| {
            "windows_restart_credential_commit_failed"
                .to_string()
        })?;

    Ok(())
}

#[cfg(target_os = "windows")]
pub fn read_windows_restart_credential_v1(
) -> Result<Zeroizing<String>, String> {
    let encrypted = fs::read(
        windows_restart_credential_path_v1()
    )
    .map_err(|_| {
        "windows_restart_credential_read_failed"
            .to_string()
    })?;

    let plaintext =
        unprotect_current_user_v1(
            &encrypted
        )?;

    let password =
        String::from_utf8(plaintext)
            .map_err(|_| {
                "windows_restart_credential_utf8_invalid"
                    .to_string()
            })?;

    if password.is_empty() {
        return Err(
            "windows_restart_credential_empty".into()
        );
    }

    Ok(Zeroizing::new(password))
}

#[cfg(test)]
#[cfg(target_os = "windows")]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trip_v1() {
        let plaintext =
            b"edgeswarm-dpapi-round-trip";

        let encrypted =
            protect_current_user_v1(
                plaintext
            )
            .unwrap();

        assert_ne!(
            encrypted,
            plaintext
        );

        let decrypted =
            unprotect_current_user_v1(
                &encrypted
            )
            .unwrap();

        assert_eq!(
            decrypted,
            plaintext
        );
    }

    #[test]
    fn tampered_dpapi_blob_fails_closed_v1() {
        let mut encrypted =
            protect_current_user_v1(
                b"edgeswarm-tamper-test"
            )
            .unwrap();

        let last =
            encrypted.len() - 1;

        encrypted[last] ^= 0xFF;

        assert!(
            unprotect_current_user_v1(
                &encrypted
            )
            .is_err()
        );
    }
}
