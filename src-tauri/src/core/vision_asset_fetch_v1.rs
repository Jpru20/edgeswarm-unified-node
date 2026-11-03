use crate::{
    adapters,
    core::{
        auth_client::SupabaseAuthClient,
        production_task_http::{
            api_base, bearer, refresh_auth, LocalAuth,
        },
        task_client::TaskEnvelope,
        vision_transport_v1::{
            validate_vision_task_contract_v1,
            VisionAssetRefV1,
        },
    },
};
use reqwest::{
    blocking::Client,
    header::CONTENT_TYPE,
    redirect::Policy,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct PreparedVisionAssetV1 {
    pub asset_id: String,
    pub media_type: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct PreparedVisionAssetsV1 {
    pub root: PathBuf,
    pub assets: Vec<PreparedVisionAssetV1>,
}

impl Drop for PreparedVisionAssetsV1 {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn extension_v1(media_type: &str) -> Result<&'static str, String> {
    match media_type {
        "image/jpeg" => Ok("jpg"),
        "image/png" => Ok("png"),
        "image/webp" => Ok("webp"),
        _ => Err("vision_asset_media_type_unsupported".into()),
    }
}

fn endpoint_from_base_v1(base: &str, asset_id: &str) -> String {
    format!(
        "{}/v1/vision/assets/{}",
        base.trim_end_matches('/'),
        asset_id
    )
}

fn task_cache_key_v1(task: &TaskEnvelope) -> String {
    let mut source = task.task_id_text();

    if let Some(attempt) = task.attempt_id.as_deref() {
        source.push(':');
        source.push_str(attempt);
    }

    Sha256::digest(source.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .take(24)
        .collect()
}

fn write_verified_asset_v1<R: Read>(
    mut input: R,
    asset: &VisionAssetRefV1,
    destination: &Path,
) -> Result<u64, String> {
    let temporary = destination.with_extension("download");

    let mut output =
        fs::File::create(&temporary)
            .map_err(|_| "vision_asset_file_create_failed".to_string())?;

    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];

    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| "vision_asset_read_failed".to_string())?;

        if read == 0 {
            break;
        }

        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "vision_asset_size_overflow".to_string())?;

        if total > asset.size_bytes {
            drop(output);
            let _ = fs::remove_file(&temporary);
            return Err("vision_asset_size_mismatch".into());
        }

        digest.update(&buffer[..read]);

        output
            .write_all(&buffer[..read])
            .map_err(|_| "vision_asset_write_failed".to_string())?;
    }

    if total != asset.size_bytes {
        drop(output);
        let _ = fs::remove_file(&temporary);
        return Err("vision_asset_size_mismatch".into());
    }

    output
        .sync_all()
        .map_err(|_| "vision_asset_sync_failed".to_string())?;

    drop(output);

    let actual = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    if !actual.eq_ignore_ascii_case(&asset.sha256) {
        let _ = fs::remove_file(&temporary);
        return Err("vision_asset_sha256_mismatch".into());
    }

    fs::rename(&temporary, destination)
        .map_err(|_| "vision_asset_commit_failed".to_string())?;

    Ok(total)
}

fn fetch_one_v1(
    http: &Client,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    task_id: &str,
    hardware_id: &str,
    asset: &VisionAssetRefV1,
    destination: &Path,
) -> Result<PreparedVisionAssetV1, String> {
    let endpoint =
        endpoint_from_base_v1(&api_base(), &asset.asset_id);

    for auth_try in 0..2 {
        let mut response = http
            .get(&endpoint)
            .header(
                "authorization",
                bearer(&auth.access_token)?,
            )
            .query(&[
                ("taskId", task_id),
                ("hardwareId", hardware_id),
            ])
            .send()
            .map_err(|_| "vision_asset_network_failed".to_string())?;

        let status = response.status().as_u16();

        if status == 401 && auth_try == 0 {
            let stale = auth.access_token.clone();

            *auth = refresh_auth(
                auth_client,
                &stale,
            )?;

            continue;
        }

        if !(200..300).contains(&status) {
            return Err(format!("vision_asset_http_{status}"));
        }

        let actual_media = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();

        if actual_media != asset.media_type.to_ascii_lowercase() {
            return Err("vision_asset_content_type_mismatch".into());
        }

        if let Some(length) = response.content_length() {
            if length != asset.size_bytes {
                return Err("vision_asset_content_length_mismatch".into());
            }
        }

        let size =
            write_verified_asset_v1(
                &mut response,
                asset,
                destination,
            )?;

        return Ok(PreparedVisionAssetV1 {
            asset_id: asset.asset_id.clone(),
            media_type: asset.media_type.clone(),
            sha256: asset.sha256.clone(),
            size_bytes: size,
            path: destination.to_path_buf(),
        });
    }

    Err("vision_asset_auth_failed_after_refresh".into())
}

pub fn prepare_vision_assets_v1(
    task: &TaskEnvelope,
    auth_client: &SupabaseAuthClient,
    auth: &mut LocalAuth,
    hardware_id: &str,
) -> Result<Option<PreparedVisionAssetsV1>, String> {
    let contract = match task.vision_input.as_ref() {
        Some(contract) => contract,
        None => return Ok(None),
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "vision_asset_clock_invalid".to_string())?
        .as_secs();

    validate_vision_task_contract_v1(
        task.required_model.as_deref(),
        task.company_model.is_some(),
        Some(contract),
        now,
    )?;

    let root = adapters::app_data_dir()
        .join("vision-input-v1")
        .join(task_cache_key_v1(task));

    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|_| "vision_asset_stale_cleanup_failed".to_string())?;
    }

    fs::create_dir_all(&root)
        .map_err(|_| "vision_asset_directory_failed".to_string())?;

    let http = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .redirect(Policy::none())
        .build()
        .map_err(|_| "vision_asset_http_client_failed".to_string())?;

    let task_id = task.task_id_text();
    let mut prepared = Vec::new();

    for asset in &contract.assets {
        let extension = extension_v1(&asset.media_type)?;
        let destination =
            root.join(format!("{}.{}", asset.asset_id, extension));

        match fetch_one_v1(
            &http,
            auth_client,
            auth,
            &task_id,
            hardware_id,
            asset,
            &destination,
        ) {
            Ok(value) => prepared.push(value),

            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        }
    }

    Ok(Some(PreparedVisionAssetsV1 {
        root,
        assets: prepared,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset_v1(
        bytes: &[u8],
    ) -> VisionAssetRefV1 {
        VisionAssetRefV1 {
            asset_id: "img_test".into(),
            media_type: "image/png".into(),
            sha256: Sha256::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            size_bytes: bytes.len() as u64,
            expires_at_unix: u64::MAX - 1,
        }
    }

    #[test]
    fn endpoint_is_swarm_owned_and_asset_id_based_v171() {
        assert_eq!(
            endpoint_from_base_v1(
                "https://api.edgeswarm.io/",
                "img_test",
            ),
            "https://api.edgeswarm.io/v1/vision/assets/img_test"
        );
    }

    #[test]
    fn verified_asset_writer_accepts_exact_bytes_v171() {
        let bytes = b"swarm-vision-test";
        let asset = asset_v1(bytes);

        let root = std::env::temp_dir()
            .join("swarm-vision-fetch-pass-v171");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let destination = root.join("img_test.png");

        let written =
            write_verified_asset_v1(
                &bytes[..],
                &asset,
                &destination,
            )
            .unwrap();

        assert_eq!(written, bytes.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), bytes);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verified_asset_writer_rejects_hash_mismatch_v171() {
        let bytes = b"swarm-vision-test";
        let mut asset = asset_v1(bytes);
        asset.sha256 = "a".repeat(64);

        let root = std::env::temp_dir()
            .join("swarm-vision-fetch-hash-v171");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let destination = root.join("img_test.png");

        assert_eq!(
            write_verified_asset_v1(
                &bytes[..],
                &asset,
                &destination,
            )
            .unwrap_err(),
            "vision_asset_sha256_mismatch"
        );

        assert!(!destination.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verified_asset_writer_rejects_size_mismatch_v171() {
        let bytes = b"swarm-vision-test";
        let mut asset = asset_v1(bytes);
        asset.size_bytes += 1;

        let root = std::env::temp_dir()
            .join("swarm-vision-fetch-size-v171");

        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let destination = root.join("img_test.png");

        assert_eq!(
            write_verified_asset_v1(
                &bytes[..],
                &asset,
                &destination,
            )
            .unwrap_err(),
            "vision_asset_size_mismatch"
        );

        assert!(!destination.exists());

        let _ = fs::remove_dir_all(root);
    }
}
