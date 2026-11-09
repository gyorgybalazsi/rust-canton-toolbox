use crate::lf_protobuf::com::daml::daml_lf::Archive;
use anyhow::{Context, Result};
use prost::Message;
use std::fs::File;
use std::io::{Cursor, Read};
use zip::ZipArchive;

pub fn archive_from_dar(dar_path: &str) -> Result<Archive> {
    let mut file =
        File::open(dar_path).with_context(|| format!("Failed to open DAR file '{}'", dar_path))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .with_context(|| format!("Failed to read DAR file '{}'", dar_path))?;

    let mut archive = ZipArchive::new(Cursor::new(buf))
        .with_context(|| format!("Failed to open zip archive '{}'", dar_path))?;

    // Parse META-INF/MANIFEST.MF to find Main-Dalf
    let main_dalf = {
        let mut manifest = archive
            .by_name("META-INF/MANIFEST.MF")
            .with_context(|| "Failed to find META-INF/MANIFEST.MF in archive")?;
        let mut manifest_str = String::new();
        manifest
            .read_to_string(&mut manifest_str)
            .with_context(|| "Failed to read META-INF/MANIFEST.MF")?;

        parse_manifest_main_dalf(&manifest_str).context("Main-Dalf not found in MANIFEST.MF")?
    };

    let mut dalf_file = archive
        .by_name(&main_dalf)
        .with_context(|| format!("Failed to find DALF file '{}' in archive", main_dalf))?;
    let mut dalf_bytes = Vec::new();
    dalf_file
        .read_to_end(&mut dalf_bytes)
        .with_context(|| format!("Failed to read DALF file '{}'", main_dalf))?;

    Archive::decode(&*dalf_bytes)
        .with_context(|| format!("Failed to decode Archive from '{}'", main_dalf))
}

pub struct RawDalf {
    pub zip_entry_name: String,
    pub bytes: Vec<u8>,
}

/// Extracts all DALF bytes from a DAR ZIP.
/// Returns (main_dalf_path, all_dalfs).
pub fn extract_dalfs_from_dar(dar_path: &str) -> Result<(String, Vec<RawDalf>)> {
    let mut file =
        File::open(dar_path).with_context(|| format!("Failed to open DAR file '{}'", dar_path))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .with_context(|| format!("Failed to read DAR file '{}'", dar_path))?;

    let mut zip = ZipArchive::new(Cursor::new(buf))
        .with_context(|| format!("Failed to open zip archive '{}'", dar_path))?;

    let main_dalf_path = {
        let mut manifest = zip
            .by_name("META-INF/MANIFEST.MF")
            .with_context(|| "Failed to find META-INF/MANIFEST.MF in archive")?;
        let mut manifest_str = String::new();
        manifest
            .read_to_string(&mut manifest_str)
            .with_context(|| "Failed to read META-INF/MANIFEST.MF")?;
        parse_manifest_main_dalf(&manifest_str).context("Main-Dalf not found in MANIFEST.MF")?
    };

    let mut dalfs = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .with_context(|| format!("Failed to read ZIP entry at index {}", i))?;
        if entry.name().ends_with(".dalf") {
            let name = entry.name().to_string();
            let mut bytes = Vec::new();
            entry
                .read_to_end(&mut bytes)
                .with_context(|| format!("Failed to read DALF '{}'", name))?;
            dalfs.push(RawDalf {
                zip_entry_name: name,
                bytes,
            });
        }
    }

    Ok((main_dalf_path, dalfs))
}

fn parse_manifest_main_dalf(manifest_str: &str) -> Option<String> {
    let mut key = String::new();
    let mut value = String::new();
    let mut found = false;

    for line in manifest_str.lines() {
        if line.starts_with(' ') {
            value.push_str(line.trim_start());
        } else {
            if key == "Main-Dalf" {
                found = true;
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                key = k.trim().to_string();
                value = v.trim().to_string();
            } else {
                key.clear();
                value.clear();
            }
        }
    }
    if key == "Main-Dalf" || found {
        Some(value)
    } else {
        None
    }
}
