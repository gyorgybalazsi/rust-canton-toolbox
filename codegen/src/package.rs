use crate::archive::{archive_from_dar, extract_dalfs_from_dar};
use crate::lf_protobuf::com::daml::daml_lf::{Archive, ArchivePayload, archive_payload};
use crate::lf_protobuf::com::daml::daml_lf_2::Package;
use anyhow::{Context, Result};
use prost::Message;
use std::collections::HashMap;
use tracing::warn;

pub struct ParsedDar {
    pub main_package_id: String,
    pub packages: HashMap<String, Package>,
}

/// Parses a single DAR file, extracting all DamlLf2 packages.
pub fn parse_dar(dar_path: &str) -> Result<ParsedDar> {
    let (main_dalf_path, raw_dalfs) = extract_dalfs_from_dar(dar_path)?;
    let mut packages = HashMap::new();
    let mut main_package_id = String::new();

    for raw in &raw_dalfs {
        let archive = Archive::decode(&*raw.bytes)
            .with_context(|| format!("Failed to decode Archive from '{}'", raw.zip_entry_name))?;

        let payload = ArchivePayload::decode(&*archive.payload).with_context(|| {
            format!(
                "Failed to decode ArchivePayload from '{}'",
                raw.zip_entry_name
            )
        })?;

        match payload.sum {
            Some(archive_payload::Sum::DamlLf2(dalf_bytes)) => {
                let package = Package::decode(&*dalf_bytes).with_context(|| {
                    format!("Failed to decode Package from '{}'", raw.zip_entry_name)
                })?;
                let pkg_id = archive.hash.clone();
                if raw.zip_entry_name == main_dalf_path {
                    main_package_id = pkg_id.clone();
                }
                packages.insert(pkg_id, package);
            }
            _ => {
                warn!("Skipping non-DamlLf2 DALF: {}", raw.zip_entry_name);
            }
        }
    }

    Ok(ParsedDar {
        main_package_id,
        packages,
    })
}

/// Parses multiple DAR files, merging all packages.
/// Deduplicates by package hash (same hash = identical content).
/// The main_package_id comes from the first DAR.
pub fn parse_dars(dar_paths: &[&str]) -> Result<ParsedDar> {
    let mut merged = ParsedDar {
        main_package_id: String::new(),
        packages: HashMap::new(),
    };

    for (i, path) in dar_paths.iter().enumerate() {
        let dar = parse_dar(path)?;
        if i == 0 {
            merged.main_package_id = dar.main_package_id;
        }
        for (id, pkg) in dar.packages {
            merged.packages.entry(id).or_insert(pkg);
        }
    }

    Ok(merged)
}

/// Extracts the main package from a DAR (backward-compatible).
pub fn package_from_dar(path: &str) -> Result<Package> {
    let archive = archive_from_dar(path)
        .with_context(|| format!("Failed to read archive from '{}'", path))?;

    let payload = ArchivePayload::decode(&*archive.payload)
        .with_context(|| "Failed to decode ArchivePayload")?;

    if let Some(archive_payload::Sum::DamlLf2(dalf_bytes)) = payload.sum {
        let package = Package::decode(&*dalf_bytes)
            .with_context(|| "Failed to decode Package from DALF bytes")?;
        Ok(package)
    } else {
        anyhow::bail!("Expected DamlLf2 variant in ArchivePayload");
    }
}
