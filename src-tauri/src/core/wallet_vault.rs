use base64::{
    engine::general_purpose::STANDARD,
    Engine,
};
use ring::{
    aead,
    pbkdf2,
    rand::{SecureRandom, SystemRandom},
};
use std::num::NonZeroU32;

const PBKDF2_ITERATIONS: u32 = 100_000;
const AES_GCM_NONCE_LEN: usize = 12;

fn derive_key(
    password: &str,
    email: &str,
) -> [u8; 32] {
    let mut salt = [0u8; 16];

    let email_bytes = email.as_bytes();
    let copy_len =
        email_bytes.len().min(salt.len());

    salt[..copy_len]
        .copy_from_slice(
            &email_bytes[..copy_len]
        );

    let mut key = [0u8; 32];

    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(
            PBKDF2_ITERATIONS
        )
        .expect("nonzero PBKDF2 iterations"),
        &salt,
        password.as_bytes(),
        &mut key,
    );

    key
}

pub fn encrypt(
    private_key: &str,
    password: &str,
    email: &str,
) -> Result<String, String> {
    let rng = SystemRandom::new();
    let mut nonce =
        [0u8; AES_GCM_NONCE_LEN];

    rng.fill(&mut nonce)
        .map_err(|_| {
            "wallet_nonce_generation_failed"
                .to_string()
        })?;

    encrypt_with_nonce(
        private_key,
        password,
        email,
        nonce,
    )
}

pub fn encrypt_with_nonce(
    private_key: &str,
    password: &str,
    email: &str,
    nonce_bytes: [u8; AES_GCM_NONCE_LEN],
) -> Result<String, String> {
    let key_bytes =
        derive_key(password, email);

    let unbound =
        aead::UnboundKey::new(
            &aead::AES_256_GCM,
            &key_bytes,
        )
        .map_err(|_| {
            "wallet_cipher_init_failed"
                .to_string()
        })?;

    let key =
        aead::LessSafeKey::new(unbound);

    let nonce =
        aead::Nonce::assume_unique_for_key(
            nonce_bytes
        );

    let mut ciphertext =
        private_key.as_bytes().to_vec();

    key.seal_in_place_append_tag(
        nonce,
        aead::Aad::empty(),
        &mut ciphertext,
    )
    .map_err(|_| {
        "wallet_encrypt_failed".to_string()
    })?;

    let mut payload =
        Vec::with_capacity(
            AES_GCM_NONCE_LEN
                + ciphertext.len(),
        );

    payload.extend_from_slice(
        &nonce_bytes
    );
    payload.extend_from_slice(
        &ciphertext
    );

    Ok(STANDARD.encode(payload))
}

pub fn decrypt(
    encrypted_payload: &str,
    password: &str,
    email: &str,
) -> Result<String, String> {
    let payload =
        STANDARD
            .decode(encrypted_payload)
            .map_err(|_| {
                "wallet_payload_base64_invalid"
                    .to_string()
            })?;

    if payload.len()
        < AES_GCM_NONCE_LEN
            + aead::AES_256_GCM.tag_len()
    {
        return Err(
            "wallet_payload_too_short".into()
        );
    }

    let mut nonce_bytes =
        [0u8; AES_GCM_NONCE_LEN];

    nonce_bytes.copy_from_slice(
        &payload[..AES_GCM_NONCE_LEN]
    );

    let mut ciphertext =
        payload[AES_GCM_NONCE_LEN..]
            .to_vec();

    let key_bytes =
        derive_key(password, email);

    let unbound =
        aead::UnboundKey::new(
            &aead::AES_256_GCM,
            &key_bytes,
        )
        .map_err(|_| {
            "wallet_cipher_init_failed"
                .to_string()
        })?;

    let key =
        aead::LessSafeKey::new(unbound);

    let plaintext =
        key.open_in_place(
            aead::Nonce::
                assume_unique_for_key(
                    nonce_bytes
                ),
            aead::Aad::empty(),
            &mut ciphertext,
        )
        .map_err(|_| {
            "wallet_decrypt_failed".to_string()
        })?;

    String::from_utf8(
        plaintext.to_vec()
    )
    .map_err(|_| {
        "wallet_plaintext_invalid_utf8"
            .to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_windows_wallet_vault_vector() {
        let encrypted =
            encrypt_with_nonce(
                "0x0123456789abcdef",
                "CorrectHorseBatteryStaple!",
                "wallet-test@example.com",
                [
                    0x00, 0x01, 0x02, 0x03,
                    0x04, 0x05, 0x06, 0x07,
                    0x08, 0x09, 0x0a, 0x0b,
                ],
            )
            .unwrap();

        assert_eq!(
            encrypted,
            "AAECAwQFBgcICQoLpgH0yD2grVV59SynqgWk4V2l1Xc4a5E5d1QxdFUKSG43oQ=="
        );

        let decrypted =
            decrypt(
                &encrypted,
                "CorrectHorseBatteryStaple!",
                "wallet-test@example.com",
            )
            .unwrap();

        assert_eq!(
            decrypted,
            "0x0123456789abcdef"
        );
    }

    #[test]
    fn wrong_password_fails_closed() {
        let encrypted =
            encrypt_with_nonce(
                "synthetic-private-key",
                "correct-password",
                "wallet-test@example.com",
                [7u8; 12],
            )
            .unwrap();

        assert!(
            decrypt(
                &encrypted,
                "wrong-password",
                "wallet-test@example.com",
            )
            .is_err()
        );
    }
}
