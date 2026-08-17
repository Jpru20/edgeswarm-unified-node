use k256::ecdsa::{
    RecoveryId,
    Signature,
    SigningKey,
    VerifyingKey,
};
use sha3::{Digest, Keccak256};

fn signing_key(private_key: &str) -> Result<SigningKey, String> {
    let value = private_key
        .trim()
        .strip_prefix("0x")
        .unwrap_or(private_key.trim());

    if value.len() != 64 {
        return Err("result_private_key_invalid_length".into());
    }

    let mut bytes = [0u8; 32];

    for index in 0..32 {
        bytes[index] = u8::from_str_radix(
            &value[index * 2..index * 2 + 2],
            16,
        )
        .map_err(|_| "result_private_key_invalid_hex".to_string())?;
    }

    SigningKey::from_bytes((&bytes).into())
        .map_err(|_| "result_private_key_invalid".to_string())
}

fn ethereum_message_digest(message: &str) -> Keccak256 {
    let prefix = format!(
        "\x19Ethereum Signed Message:\n{}",
        message.as_bytes().len()
    );

    let mut digest = Keccak256::new();
    digest.update(prefix.as_bytes());
    digest.update(message.as_bytes());
    digest
}

pub fn result_message(
    task_id: &str,
    score: u16,
    file_hash: &str,
    hardware_id: &str,
) -> String {
    format!(
        "Task:{task_id}|Score:{score}|Hash:{file_hash}|HW:{hardware_id}"
    )
}

pub fn sign_result(
    task_id: &str,
    score: u16,
    file_hash: &str,
    hardware_id: &str,
    private_key: &str,
) -> Result<String, String> {
    let message = result_message(
        task_id,
        score,
        file_hash,
        hardware_id,
    );

    let key = signing_key(private_key)?;

    let (signature, recovery_id) = key
        .sign_digest_recoverable(
            ethereum_message_digest(&message)
        )
        .map_err(|_| "result_signing_failed".to_string())?;

    let mut output = Vec::with_capacity(65);
    output.extend_from_slice(&signature.to_bytes());

    // Match eth_account Account.sign_message().signature.hex():
    // final recovery byte is Ethereum v=27/28.
    output.push(27 + recovery_id.to_byte());

    Ok(format!(
        "0x{}",
        output
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

pub fn recover_result_signer(
    task_id: &str,
    score: u16,
    file_hash: &str,
    hardware_id: &str,
    signature_hex: &str,
) -> Result<String, String> {
    let raw = signature_hex
        .trim()
        .strip_prefix("0x")
        .unwrap_or(signature_hex.trim());

    if raw.len() != 130 {
        return Err("result_signature_invalid_length".into());
    }

    let mut bytes = [0u8; 65];

    for index in 0..65 {
        bytes[index] = u8::from_str_radix(
            &raw[index * 2..index * 2 + 2],
            16,
        )
        .map_err(|_| "result_signature_invalid_hex".to_string())?;
    }

    let signature = Signature::try_from(&bytes[..64])
        .map_err(|_| "result_signature_invalid".to_string())?;

    let recovery_byte = match bytes[64] {
        27 | 28 => bytes[64] - 27,
        0 | 1 => bytes[64],
        _ => return Err("result_signature_recovery_id_invalid".into()),
    };

    let recovery_id = RecoveryId::try_from(recovery_byte)
        .map_err(|_| "result_signature_recovery_id_invalid".to_string())?;

    let message = result_message(
        task_id,
        score,
        file_hash,
        hardware_id,
    );

    let verifying_key = VerifyingKey::recover_from_digest(
        ethereum_message_digest(&message),
        &signature,
        recovery_id,
    )
    .map_err(|_| "result_signature_recovery_failed".to_string())?;

    let encoded = verifying_key.to_encoded_point(false);
    let public = encoded.as_bytes();

    let digest = Keccak256::digest(&public[1..]);
    let address = &digest[digest.len() - 20..];

    Ok(format!(
        "0x{}",
        address
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::wallet_account::DeviceWallet;

    #[test]
    fn eip191_result_signature_recovers_wallet() {
        let private_key =
            "0000000000000000000000000000000000000000000000000000000000000001";

        let wallet =
            DeviceWallet::from_private_key(private_key)
                .unwrap();

        let signature = sign_result(
            "1234",
            100,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            private_key,
        )
        .unwrap();

        let recovered = recover_result_signer(
            "1234",
            100,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            &signature,
        )
        .unwrap();

        assert_eq!(
            recovered.to_lowercase(),
            wallet.wallet_address().to_lowercase()
        );

        assert_eq!(signature.len(), 132);
    }
}
