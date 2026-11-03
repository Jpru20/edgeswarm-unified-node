use serde::Deserialize;
use std::collections::HashSet;

pub const VISION_INPUT_CONTRACT_V1:
    &str =
    "swarm-vision-input-v1";

pub const VISION_ASSET_TRANSPORT_V1:
    &str =
    "swarm-api-v1";

pub const MAX_VISION_ASSETS_V1:
    usize = 4;

pub const MAX_VISION_ASSET_BYTES_V1:
    u64 =
    20 * 1024 * 1024;

pub const MAX_VISION_TOTAL_BYTES_V1:
    u64 =
    40 * 1024 * 1024;

pub const MAX_VISION_EXPIRY_SECONDS_V1:
    u64 =
    24 * 60 * 60;

#[derive(
    Debug,
    Clone,
    Deserialize,
    PartialEq,
    Eq,
)]
#[serde(rename_all = "camelCase")]
pub struct VisionAssetRefV1 {
    pub asset_id: String,
    pub media_type: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub expires_at_unix: u64,
}

#[derive(
    Debug,
    Clone,
    Deserialize,
    PartialEq,
    Eq,
)]
#[serde(rename_all = "camelCase")]
pub struct VisionInputContractV1 {
    pub version: String,
    pub transport: String,

    #[serde(default)]
    pub primary_asset_id:
        Option<String>,

    pub assets:
        Vec<VisionAssetRefV1>,
}

fn valid_asset_id_v1(
    value: &str,
) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'-' | b'_'
                    )
            })
}

fn valid_sha256_v1(
    value: &str,
) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| {
                byte.is_ascii_hexdigit()
            })
}

fn supported_media_type_v1(
    value: &str,
) -> bool {
    matches!(
        value,
        "image/jpeg"
            | "image/png"
            | "image/webp"
    )
}

pub fn required_model_is_vision_v1(
    required_model: Option<&str>,
) -> bool {
    required_model
        .unwrap_or("")
        .trim()
        .starts_with(
            "Vision-Inference"
        )
}

pub fn validate_vision_input_contract_v1(
    contract:
        &VisionInputContractV1,
    now_unix: u64,
) -> Result<(), String> {
    if contract.version
        != VISION_INPUT_CONTRACT_V1
    {
        return Err(
            "vision_input_version_unsupported"
                .into()
        );
    }

    if contract.transport
        != VISION_ASSET_TRANSPORT_V1
    {
        return Err(
            "vision_input_transport_unsupported"
                .into()
        );
    }

    if contract.assets.is_empty()
        || contract.assets.len()
            > MAX_VISION_ASSETS_V1
    {
        return Err(
            "vision_input_asset_count_invalid"
                .into()
        );
    }

    let mut ids =
        HashSet::<String>::new();

    let mut total_bytes =
        0u64;

    for asset in &contract.assets {
        if !valid_asset_id_v1(
            &asset.asset_id,
        ) {
            return Err(
                "vision_asset_id_invalid"
                    .into()
            );
        }

        if !ids.insert(
            asset.asset_id.clone(),
        ) {
            return Err(
                "vision_asset_id_duplicate"
                    .into()
            );
        }

        if !supported_media_type_v1(
            &asset.media_type,
        ) {
            return Err(
                "vision_asset_media_type_unsupported"
                    .into()
            );
        }

        if !valid_sha256_v1(
            &asset.sha256,
        ) {
            return Err(
                "vision_asset_sha256_invalid"
                    .into()
            );
        }

        if asset.size_bytes == 0
            || asset.size_bytes
                > MAX_VISION_ASSET_BYTES_V1
        {
            return Err(
                "vision_asset_size_invalid"
                    .into()
            );
        }

        total_bytes =
            total_bytes
                .checked_add(
                    asset.size_bytes,
                )
                .ok_or_else(|| {
                    "vision_asset_size_overflow"
                        .to_string()
                })?;

        if total_bytes
            > MAX_VISION_TOTAL_BYTES_V1
        {
            return Err(
                "vision_input_total_size_invalid"
                    .into()
            );
        }

        if asset.expires_at_unix
            <= now_unix
        {
            return Err(
                "vision_asset_expired"
                    .into()
            );
        }

        if asset.expires_at_unix
            > now_unix
                .saturating_add(
                    MAX_VISION_EXPIRY_SECONDS_V1
                )
        {
            return Err(
                "vision_asset_expiry_too_far"
                    .into()
            );
        }
    }

    if let Some(primary) =
        contract
            .primary_asset_id
            .as_deref()
    {
        if !ids.contains(primary) {
            return Err(
                "vision_primary_asset_missing"
                    .into()
            );
        }
    }

    Ok(())
}

pub fn validate_vision_task_contract_v1(
    required_model: Option<&str>,
    company_model_present: bool,
    vision_input:
        Option<&VisionInputContractV1>,
    now_unix: u64,
) -> Result<(), String> {
    let vision_required =
        required_model_is_vision_v1(
            required_model,
        );

    match (
        vision_required,
        vision_input,
    ) {
        (false, None) =>
            return Ok(()),

        (true, None) =>
            return Err(
                "vision_input_missing"
                    .into()
            ),

        (false, Some(_)) =>
            return Err(
                "vision_input_requires_vision_model"
                    .into()
            ),

        (true, Some(contract)) => {
            if company_model_present {
                return Err(
                    "vision_company_model_unsupported_v1"
                        .into()
                );
            }

            validate_vision_input_contract_v1(
                contract,
                now_unix,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_contract(
        now: u64,
    ) -> VisionInputContractV1 {
        VisionInputContractV1 {
            version:
                VISION_INPUT_CONTRACT_V1
                    .into(),
            transport:
                VISION_ASSET_TRANSPORT_V1
                    .into(),
            primary_asset_id:
                Some(
                    "img_test_1".into()
                ),
            assets: vec![
                VisionAssetRefV1 {
                    asset_id:
                        "img_test_1".into(),
                    media_type:
                        "image/jpeg".into(),
                    sha256:
                        "a".repeat(64),
                    size_bytes:
                        1024,
                    expires_at_unix:
                        now + 3600,
                },
            ],
        }
    }

    #[test]
    fn valid_contract_passes_v171() {
        let now = 1000;

        validate_vision_input_contract_v1(
            &valid_contract(now),
            now,
        )
        .unwrap();
    }

    #[test]
    fn expired_asset_fails_v171() {
        let now = 1000;
        let mut contract =
            valid_contract(now);

        contract.assets[0]
            .expires_at_unix =
            now;

        assert_eq!(
            validate_vision_input_contract_v1(
                &contract,
                now,
            )
            .unwrap_err(),
            "vision_asset_expired"
        );
    }

    #[test]
    fn arbitrary_media_type_fails_v171() {
        let now = 1000;
        let mut contract =
            valid_contract(now);

        contract.assets[0]
            .media_type =
            "application/pdf".into();

        assert_eq!(
            validate_vision_input_contract_v1(
                &contract,
                now,
            )
            .unwrap_err(),
            "vision_asset_media_type_unsupported"
        );
    }

    #[test]
    fn neural_task_cannot_smuggle_vision_input_v171() {
        let now = 1000;
        let contract =
            valid_contract(now);

        assert_eq!(
            validate_vision_task_contract_v1(
                Some(
                    "Neural-Inference-30B"
                ),
                false,
                Some(&contract),
                now,
            )
            .unwrap_err(),
            "vision_input_requires_vision_model"
        );
    }

    #[test]
    fn vision_requires_contract_v171() {
        assert_eq!(
            validate_vision_task_contract_v1(
                Some(
                    "Vision-Inference-27B"
                ),
                false,
                None,
                1000,
            )
            .unwrap_err(),
            "vision_input_missing"
        );
    }

    #[test]
    fn company_adapter_not_allowed_on_vision_v1() {
        let now = 1000;
        let contract =
            valid_contract(now);

        assert_eq!(
            validate_vision_task_contract_v1(
                Some(
                    "Vision-Inference-27B"
                ),
                true,
                Some(&contract),
                now,
            )
            .unwrap_err(),
            "vision_company_model_unsupported_v1"
        );
    }
}
