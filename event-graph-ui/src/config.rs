use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Deserialize, Clone)]
pub struct AppConfig {
    pub logging: LoggingConfig,
    pub neo4j: Neo4jConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Debug, Deserialize, Clone)]
pub struct Neo4jConfig {
    pub uri: String,
    pub user: String,
    pub password: String,
}

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Matches `${VAR}` where VAR uses [A-Za-z0-9_]. Letters of either case
    // are accepted so contributors can pick natural names; convention in
    // this repo is uppercase (NEO4J_PASSWORD, etc.).
    RE.get_or_init(|| Regex::new(r"\$\{([A-Za-z0-9_]+)\}").unwrap())
}

/// Replace `${VAR}` substrings with `std::env::var(VAR)`.
///
/// `field` is a human-readable path (e.g. `"neo4j.password"`) used only to
/// produce an actionable error message when a referenced env var is unset.
/// Returns `s` unchanged when it contains no placeholders. Mirrors the
/// helper of the same name in `event-graph-indexer/src/config.rs`.
fn substitute_env(s: &str, field: &str) -> Result<String> {
    let re = placeholder_re();
    let mut out = String::with_capacity(s.len());
    let mut last_end = 0;
    for cap in re.captures_iter(s) {
        let m = cap.get(0).unwrap();
        let name = &cap[1];
        let val = std::env::var(name).with_context(|| {
            format!(
                "config field '{}' references ${{{}}} but the env var is not set. \
             Try running under `op run --env-file=secrets.env.1password -- ...`",
                field, name
            )
        })?;
        out.push_str(&s[last_end..m.start()]);
        out.push_str(&val);
        last_end = m.end();
    }
    out.push_str(&s[last_end..]);
    Ok(out)
}

pub fn read_config<P: AsRef<Path>>(path: P) -> Result<AppConfig> {
    let s = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file '{}'", path.as_ref().display()))?;
    let mut cfg: AppConfig = toml::from_str(&s).context("failed to parse TOML config")?;
    cfg.neo4j.password = substitute_env(&cfg.neo4j.password, "neo4j.password")?;
    Ok(cfg)
}

pub fn find_and_read_config() -> Result<AppConfig> {
    let cwd_config = std::path::PathBuf::from("config").join("config.toml");
    if cwd_config.exists() {
        return read_config(&cwd_config);
    }

    if let Ok(crate_root) = std::env::var("CARGO_MANIFEST_DIR") {
        let cargo_config = std::path::PathBuf::from(&crate_root)
            .join("config")
            .join("config.toml");
        if cargo_config.exists() {
            return read_config(&cargo_config);
        }
    }

    // Try parent directory (workspace root) config
    let parent_config = std::path::PathBuf::from("../event-graph-indexer/config/config.toml");
    if parent_config.exists() {
        return read_config(&parent_config);
    }

    anyhow::bail!(
        "Could not find config.toml. Place it at ./config/config.toml or set CARGO_MANIFEST_DIR."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_env_returns_value_when_set() {
        unsafe {
            std::env::set_var("TEST_UI_SUB_BASIC", "hello");
        }
        let out = substitute_env("${TEST_UI_SUB_BASIC}", "neo4j.password").unwrap();
        assert_eq!(out, "hello");
        unsafe {
            std::env::remove_var("TEST_UI_SUB_BASIC");
        }
    }

    #[test]
    fn substitute_env_errors_when_unset() {
        unsafe {
            std::env::remove_var("TEST_UI_SUB_MISSING");
        }
        let err = substitute_env("${TEST_UI_SUB_MISSING}", "neo4j.password")
            .expect_err("must fail when env var is unset");
        let msg = format!("{:#}", err);
        assert!(msg.contains("neo4j.password"), "missing field name: {msg}");
        assert!(
            msg.contains("TEST_UI_SUB_MISSING"),
            "missing var name: {msg}"
        );
        assert!(
            msg.contains("op run --env-file=secrets.env.1password"),
            "missing op run guidance: {msg}"
        );
    }

    #[test]
    fn substitute_env_is_noop_without_placeholders() {
        let out = substitute_env("plainvalue", "x").unwrap();
        assert_eq!(out, "plainvalue");
    }

    #[test]
    fn read_config_substitutes_neo4j_password() {
        use std::io::Write;
        unsafe {
            std::env::set_var("TEST_UI_READ_NEO4J", "neo-pw");
        }
        // PID-suffixed name avoids collisions when this test crate is run in
        // parallel with another (e.g. `cargo test --workspace`).
        let tmp =
            std::env::temp_dir().join(format!("test_ui_read_config_{}.toml", std::process::id()));
        let mut f = std::fs::File::create(&tmp).unwrap();
        writeln!(
            f,
            r#"
[logging]
level = "info"

[neo4j]
uri = "neo4j://127.0.0.1:7687"
user = "neo4j"
password = "${{TEST_UI_READ_NEO4J}}"
"#
        )
        .unwrap();
        drop(f);

        let cfg = read_config(&tmp).unwrap();
        assert_eq!(cfg.neo4j.password, "neo-pw");

        std::fs::remove_file(&tmp).ok();
        unsafe {
            std::env::remove_var("TEST_UI_READ_NEO4J");
        }
    }
}
