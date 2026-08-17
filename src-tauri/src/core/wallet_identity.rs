use serde::{
    Deserialize,
    Serialize,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerWalletRow {
    pub id: serde_json::Value,
    pub hardware_id: Option<String>,
    pub private_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalletRowDecision {
    ExactDevice {
        row_index: usize,
    },
    ClaimLegacy {
        row_index: usize,
    },
    CreateDevice,
}

pub fn select_wallet_row(
    rows: &[WorkerWalletRow],
    hardware_id: &str,
) -> Result<WalletRowDecision, String> {
    let hardware_id =
        hardware_id.trim().to_lowercase();

    if hardware_id.len() != 64
        || !hardware_id
            .bytes()
            .all(|byte| {
                byte.is_ascii_digit()
                    || (b'a'..=b'f')
                        .contains(&byte)
            })
    {
        return Err(
            "wallet_hardware_id_invalid"
                .into()
        );
    }

    if let Some(index) =
        rows.iter().position(|row| {
            row.hardware_id
                .as_deref()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case(
                    &hardware_id
                )
        })
    {
        return Ok(
            WalletRowDecision::ExactDevice {
                row_index: index,
            },
        );
    }

    if let Some(index) =
        rows.iter().position(|row| {
            row.hardware_id
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
        })
    {
        return Ok(
            WalletRowDecision::ClaimLegacy {
                row_index: index,
            },
        );
    }

    Ok(WalletRowDecision::CreateDevice)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HW: &str =
        "6957eb286fb15a2813ce44699232d053d69d91be307fdf0018df1001b4eda5de";

    #[test]
    fn exact_device_wins_over_legacy() {
        let rows = vec![
            WorkerWalletRow {
                id: "legacy".into(),
                hardware_id: None,
                private_key: "encrypted-a".into(),
            },
            WorkerWalletRow {
                id: "exact".into(),
                hardware_id:
                    Some(HW.into()),
                private_key: "encrypted-b".into(),
            },
        ];

        assert_eq!(
            select_wallet_row(
                &rows,
                HW
            )
            .unwrap(),
            WalletRowDecision::ExactDevice {
                row_index: 1
            }
        );
    }

    #[test]
    fn legacy_is_claimed_before_creation() {
        let rows = vec![
            WorkerWalletRow {
                id: "legacy".into(),
                hardware_id: None,
                private_key: "encrypted".into(),
            },
        ];

        assert_eq!(
            select_wallet_row(
                &rows,
                HW
            )
            .unwrap(),
            WalletRowDecision::ClaimLegacy {
                row_index: 0
            }
        );
    }

    #[test]
    fn no_matching_row_creates_device() {
        let other =
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

        let rows = vec![
            WorkerWalletRow {
                id: "other".into(),
                hardware_id:
                    Some(other.into()),
                private_key: "encrypted".into(),
            },
        ];

        assert_eq!(
            select_wallet_row(
                &rows,
                HW
            )
            .unwrap(),
            WalletRowDecision::CreateDevice
        );
    }
}
