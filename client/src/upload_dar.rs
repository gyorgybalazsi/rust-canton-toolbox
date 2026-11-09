use anyhow::Result;
use ledger_api::v2::admin::{
    UploadDarFileRequest, package_management_service_client::PackageManagementServiceClient,
};
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Command;
use tonic::transport::Channel;
use tracing::{error, info};
use zip::ZipArchive;

/// Uploads a list of DAR files to the ledger via gRPC PackageManagementService.
/// `ledger_api` is a PathBuf to the ledger API endpoint (e.g., "http://localhost:6865").
pub async fn upload_dars(ledger_api: &Path, dar_paths: &[std::path::PathBuf]) -> Result<()> {
    let url = ledger_api.to_string_lossy().into_owned();
    let channel = Channel::from_shared(url)
        .unwrap()
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to ledger API: {}", e))?;

    let mut client = PackageManagementServiceClient::new(channel);

    for dar_path in dar_paths {
        let mut file = File::open(dar_path)?;
        let mut dar_bytes = Vec::new();
        file.read_to_end(&mut dar_bytes)?;

        let request = UploadDarFileRequest {
            dar_file: dar_bytes,
            submission_id: uuid::Uuid::new_v4().to_string(),
            vetting_change: 0,
            synchronizer_id: "".to_string(),
        };

        info!("Requesting DAR file upload: {:?}", dar_path);
        match client.upload_dar_file(request).await {
            Ok(response) => info!("DAR upload request response messsage: {:?}", response),
            Err(e) => error!("Failed to request DAR upload {:?}: {:?}", dar_path, e),
        }
    }
    Ok(())
}

pub async fn list_dars(ledger_api: &Path) -> Result<Vec<String>> {
    let url = ledger_api.to_string_lossy().into_owned();
    let channel = Channel::from_shared(url)
        .unwrap()
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to ledger API: {}", e))?;

    let mut client = PackageManagementServiceClient::new(channel);

    let response = client
        .list_known_packages(tonic::Request::new(
            ledger_api::v2::admin::ListKnownPackagesRequest {},
        ))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to list known packages: {}", e))?;

    let package_details = response.into_inner().package_details;
    let packages: Vec<String> = package_details
        .into_iter()
        .map(|detail| detail.package_id)
        .collect();
    info!("Known packages: {:?}", packages);
    Ok(packages)
}

/// Extracts the package ID from a DAR file using the `daml damlc inspect-dar` CLI.
/// Returns Ok(package_id) if successful, otherwise an error.
pub fn extract_package_id_from_dar(dar_path: &Path) -> anyhow::Result<String> {
    let output = Command::new("daml")
        .args(["damlc", "inspect-dar", dar_path.to_str().unwrap(), "--json"])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run daml damlc inspect-dar: {}", e))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "daml damlc inspect-dar failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let package_id = json
        .get("main_package_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("main_package_id not found in inspect-dar output"))?;

    Ok(package_id.to_string())
}

/// Extracts the package ID from the MANIFEST.MF file inside a DAR directory.
/// Returns Ok(package_id) if successful, otherwise an error.
pub fn package_id_from_dar(dar_path: &Path) -> anyhow::Result<String> {
    let file =
        File::open(dar_path).map_err(|e| anyhow::anyhow!("Failed to open DAR file: {}", e))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| anyhow::anyhow!("Failed to open DAR as zip archive: {}", e))?;

    // Find the index of the MANIFEST.MF file inside META-INF
    let mut manifest_index = None;
    for i in 0..archive.len() {
        let name = archive.by_index(i)?.name().to_string();
        if name.ends_with("META-INF/MANIFEST.MF") {
            manifest_index = Some(i);
            break;
        }
    }

    let mut manifest = match manifest_index {
        Some(idx) => archive.by_index(idx)?,
        None => return Err(anyhow::anyhow!("MANIFEST.MF not found in DAR")),
    };

    let reader = BufReader::new(&mut manifest);
    let mut main_dalf_line = String::new();
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("Main-Dalf:") {
            main_dalf_line.push_str(line.trim_start_matches("Main-Dalf:").trim());
        } else if !main_dalf_line.is_empty() && line.starts_with(' ') {
            // Continuation line (starts with a space)
            main_dalf_line.push_str(line.trim());
        } else if !main_dalf_line.is_empty() {
            break;
        }
    }

    // Find the package id in the main_dalf_line
    let re = regex::Regex::new(r"-([a-f0-9]{40,})/").unwrap();
    if let Some(caps) = re.captures(&main_dalf_line) {
        let package_id = caps.get(1).unwrap().as_str();
        Ok(package_id.to_string())
    } else {
        Err(anyhow::anyhow!(
            "Failed to extract package id from MANIFEST.MF"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::start_sandbox;
    use std::path::PathBuf;
    use tokio;

    #[tokio::test]
    async fn test_upload_dar_files() -> anyhow::Result<()> {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();

        let package_root = PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-interface-example")
            .join("test");
        let dar_path = package_root
            .join(".daml")
            .join("dist")
            .join("daml-interface-example-test-1.0.0.dar");
        let sandbox_port = 6865;
        let _guard = start_sandbox(package_root, dar_path, sandbox_port)
            .await
            .expect("Failed to start sandbox");

        let ledger_api = PathBuf::from(format!("http://localhost:{}", sandbox_port));
        let dar_paths = vec![
            PathBuf::from(&crate_root)
                .join("..")
                .join("_daml")
                .join("daml-interface-example")
                .join("interfaces")
                .join(".daml")
                .join("dist")
                .join("daml-interface-example-interfaces-1.0.0.dar"),
            PathBuf::from(&crate_root)
                .join("..")
                .join("_daml")
                .join("daml-interface-example")
                .join("main")
                .join(".daml")
                .join("dist")
                .join("daml-interface-example-main-1.0.0.dar"),
        ];

        upload_dars(&ledger_api, &dar_paths).await?;

        let known_packages = list_dars(&ledger_api).await?;

        for dar_path in &dar_paths {
            let package_id =
                package_id_from_dar(dar_path).expect("Failed to extract package id from DAR");

            info!("Uploaded DAR package id: {}", package_id);
            assert!(
                known_packages.iter().any(|pkg| pkg == &package_id),
                "Uploaded DAR package id '{}' not found in known packages: {:?}",
                package_id,
                known_packages
            );
        }
        Ok(())
    }
}
