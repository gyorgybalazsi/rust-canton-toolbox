use crate::jwt::KeycloakConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Top-level config file structure. Only deserializes the fields the client needs;
/// unknown keys (logging, surrealdb, etc.) are silently ignored by serde.
#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    pub active_profile: String,
    pub profiles: HashMap<String, ProfileConfig>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileConfig {
    pub ledger: LedgerConfig,
    pub keycloak: Option<KeycloakConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LedgerConfig {
    pub url: String,
    pub parties: Option<Vec<String>>,
    pub fake_jwt_user: String,
}

/// Resolved config after selecting a profile.
#[derive(Debug)]
pub struct Config {
    pub ledger: LedgerConfig,
    pub keycloak: Option<KeycloakConfig>,
}

/// Read and parse a config file, then resolve the given profile.
pub fn read_config<P: AsRef<Path>>(path: P, profile: Option<&str>) -> Result<Config> {
    let s = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file '{}'", path.as_ref().display()))?;
    let config_file: ConfigFile = toml::from_str(&s).context("failed to parse TOML config")?;
    resolve_config(config_file, profile)
}

/// Auto-discover config.toml and resolve the given profile.
///
/// Lookup order:
/// 1. `./config/config.toml` (relative to cwd)
/// 2. `./syncer/config/config.toml` (repo root fallback)
/// 3. `CARGO_MANIFEST_DIR/config/config.toml` (for `cargo run`)
pub fn read_config_from_toml(profile: Option<&str>) -> Result<Config> {
    let candidates = [
        Some(std::path::PathBuf::from("config").join("config.toml")),
        Some(
            std::path::PathBuf::from("syncer")
                .join("config")
                .join("config.toml"),
        ),
        std::env::var("CARGO_MANIFEST_DIR").ok().map(|root| {
            std::path::PathBuf::from(root)
                .join("config")
                .join("config.toml")
        }),
    ];

    for candidate in candidates.into_iter().flatten() {
        if candidate.exists() {
            return read_config(&candidate, profile);
        }
    }

    anyhow::bail!(
        "Could not find config.toml. Searched: ./config/config.toml, \
         ./syncer/config/config.toml, CARGO_MANIFEST_DIR/config/config.toml. \
         Use --config-file to specify a path."
    )
}

fn resolve_config(config_file: ConfigFile, profile_override: Option<&str>) -> Result<Config> {
    let profile_name = profile_override.unwrap_or(&config_file.active_profile);

    let profile = config_file.profiles.get(profile_name).with_context(|| {
        format!(
            "profile '{}' not found. Available profiles: {:?}",
            profile_name,
            config_file.profiles.keys().collect::<Vec<_>>()
        )
    })?;

    Ok(Config {
        ledger: profile.ledger.clone(),
        keycloak: profile.keycloak.clone(),
    })
}
