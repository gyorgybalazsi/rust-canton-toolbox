use std::error;
use std::fs;
use std::io::Error;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const ALL_PROTO_SRC_PATHS: &[&str] = &["com/digitalasset/daml/lf/archive"];
const PROTO_ROOT_PATH: &str = "resources/protobuf";
const PROTO_ARCHIVE_DIR: &str = "com/digitalasset/daml/lf/archive";
const DESIRED_VERSION_FILE: &str = "daml-sdk-version";
const PROTO_FILES: &[&str] = &["daml_lf.proto", "daml_lf2.proto"];
const CANTON_RAW_URL: &str = "https://raw.githubusercontent.com/digital-asset/canton";
const CANTON_PROTO_DIR: &str =
    "community/daml-lf/archive/src/main/protobuf/com/digitalasset/daml/lf/archive";

fn main() -> Result<(), Box<dyn error::Error>> {
    ensure_proto_version()?;
    let all_protos = get_all_protos(ALL_PROTO_SRC_PATHS)?;
    prost_build::compile_protos(all_protos.as_slice(), vec![PROTO_ROOT_PATH].as_slice())?;
    Ok(())
}

/// Reads the desired SDK version from `codegen/daml-sdk-version`,
/// compares it to the installed proto version (stored in OUT_DIR),
/// and downloads the correct proto if they differ.
fn ensure_proto_version() -> Result<(), Box<dyn error::Error>> {
    let desired_version_path = Path::new(DESIRED_VERSION_FILE);
    let out_dir = std::env::var("OUT_DIR")?;
    let installed_version_path = Path::new(&out_dir).join("installed-sdk-version");

    // Re-run if the desired version file changes
    println!("cargo:rerun-if-changed={}", desired_version_path.display());

    let desired = match fs::read_to_string(desired_version_path) {
        Ok(v) => v.trim().to_string(),
        Err(_) => {
            println!(
                "cargo:warning=No {} file found — skipping proto update. \
                Create it with the Canton version (e.g. 3.6.1) to enable auto-download.",
                DESIRED_VERSION_FILE
            );
            return Ok(());
        }
    };

    let installed = fs::read_to_string(&installed_version_path)
        .map(|v| v.trim().to_string())
        .unwrap_or_default();

    if desired == installed {
        return Ok(());
    }

    println!(
        "cargo:warning=DAML LF proto: updating {} -> {}...",
        if installed.is_empty() {
            "none".to_string()
        } else {
            installed
        },
        desired
    );

    download_and_install_proto(&desired)?;

    fs::write(&installed_version_path, format!("{}\n", desired))?;
    println!("cargo:warning=DAML LF proto updated to {}", desired);

    Ok(())
}

fn download_and_install_proto(version: &str) -> Result<(), Box<dyn error::Error>> {
    // Daml-LF protos live in the Canton repository since Canton 3.5
    for proto_file in PROTO_FILES {
        let url = format!(
            "{}/v{}/{}/{}",
            CANTON_RAW_URL, version, CANTON_PROTO_DIR, proto_file
        );
        let target_path = Path::new(PROTO_ROOT_PATH)
            .join(PROTO_ARCHIVE_DIR)
            .join(proto_file);

        // Download using curl
        let status = Command::new("curl")
            .args(["-fsSL", "-o"])
            .arg(&target_path)
            .arg(&url)
            .status()?;

        if !status.success() {
            return Err(format!(
                "Failed to download {} from {}. \
                Check that Canton version {} exists at https://github.com/digital-asset/canton/tags",
                proto_file, url, version
            ).into());
        }
    }

    Ok(())
}

fn get_all_protos(src_paths: &[&str]) -> Result<Vec<PathBuf>, Error> {
    let mut protos = Vec::new();
    for path in src_paths {
        let dir = Path::new(path);
        let files = get_protos_from_dir(dir)?;
        protos.extend(files);
    }
    Ok(protos)
}

fn get_protos_from_dir(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    fs::read_dir(Path::new(PROTO_ROOT_PATH).join(dir))?
        .filter_map(|entry| match entry {
            Ok(d) => match d.path().extension() {
                Some(a) if a == "proto" => Some(Ok(d.path())),
                _ => None,
            },
            Err(e) => Some(Err(e)),
        })
        .collect()
}
