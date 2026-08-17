use k256::{elliptic_curve::sec1::ToEncodedPoint, SecretKey};
use rand_core::OsRng;
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

pub struct DeviceWallet {
    private_key: Zeroizing<String>,
    wallet_address: String,
}

impl DeviceWallet {
    pub fn generate() -> Result<Self, String> {
        let secret = SecretKey::random(&mut OsRng);
        Self::from_secret(&secret)
    }

    pub fn from_private_key(value: &str) -> Result<Self, String> {
        let normalized = value
            .trim()
            .strip_prefix("0x")
            .unwrap_or(value.trim());

        if normalized.len() != 64 {
            return Err("wallet_private_key_invalid_length".into());
        }

        let mut bytes = [0u8; 32];

        for index in 0..32 {
            bytes[index] = u8::from_str_radix(
                &normalized[index * 2..index * 2 + 2],
                16,
            )
            .map_err(|_| "wallet_private_key_invalid_hex".to_string())?;
        }

        let secret = SecretKey::from_slice(&bytes)
            .map_err(|_| "wallet_private_key_invalid".to_string())?;

        Self::from_secret(&secret)
    }

    fn from_secret(secret: &SecretKey) -> Result<Self, String> {
        let encoded = secret
            .public_key()
            .to_encoded_point(false);

        let public = encoded.as_bytes();

        if public.len() != 65 || public[0] != 0x04 {
            return Err("wallet_public_key_invalid".into());
        }

        let digest = Keccak256::digest(&public[1..]);
        let address = &digest[digest.len() - 20..];

        let lower = address
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let wallet_address = checksum_address(&lower)?;

        let private_key = format!(
            "0x{}",
            secret
                .to_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );

        Ok(Self {
            private_key: Zeroizing::new(private_key),
            wallet_address,
        })
    }

    pub fn private_key(&self) -> &str {
        self.private_key.as_str()
    }

    pub fn wallet_address(&self) -> &str {
        &self.wallet_address
    }
}

fn checksum_address(lower: &str) -> Result<String, String> {
    if lower.len() != 40 {
        return Err("wallet_address_invalid_length".into());
    }

    let hash = Keccak256::digest(lower.as_bytes());
    let mut result = String::from("0x");

    for (index, byte) in lower.bytes().enumerate() {
        if byte.is_ascii_digit() {
            result.push(byte as char);
            continue;
        }

        let hash_byte = hash[index / 2];
        let nibble = if index % 2 == 0 {
            hash_byte >> 4
        } else {
            hash_byte & 0x0f
        };

        if nibble >= 8 {
            result.push((byte as char).to_ascii_uppercase());
        } else {
            result.push(byte as char);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_ethereum_account_vector() {
        let wallet = DeviceWallet::from_private_key(
            "0000000000000000000000000000000000000000000000000000000000000001"
        )
        .unwrap();

        assert_eq!(
            wallet.wallet_address(),
            "0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf"
        );
    }

    #[test]
    fn generated_account_round_trips() {
        let generated = DeviceWallet::generate().unwrap();

        let recovered =
            DeviceWallet::from_private_key(
                generated.private_key()
            )
            .unwrap();

        assert_eq!(
            generated.wallet_address(),
            recovered.wallet_address()
        );
    }
}
