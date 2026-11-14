use anyhow::{Context, Result};
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

/// Top-level config file structure with profile support
#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    pub logging: LoggingConfig,
    /// SurrealDB connection and batching settings. Every connection field can
    /// also come from a `SURREAL_*` environment variable, which wins.
    #[serde(default)]
    pub surrealdb: SurrealConfig,
    /// The active profile name (can be overridden via CLI)
    pub active_profile: String,
    /// Named profiles containing ledger and keycloak settings
    pub profiles: HashMap<String, ProfileConfig>,
}

fn placeholder_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Matches `${VAR}` where VAR uses [A-Za-z0-9_]. Letters of either case
    // are accepted so contributors can pick natural names; convention in
    // this repo is uppercase (SURREAL_PASSWORD, MAINNET_CLIENT_SECRET, etc.).
    RE.get_or_init(|| Regex::new(r"\$\{([A-Za-z0-9_]+)\}").unwrap())
}

/// Replace `${VAR}` substrings with `std::env::var(VAR)`.
///
/// `field` is a human-readable path (e.g. `"keycloak.client_secret"`) used
/// only to produce an actionable error message when a referenced env var is
/// unset. Returns `s` unchanged when it contains no placeholders.
fn substitute_env(s: &str, field: &str) -> anyhow::Result<String> {
    let re = placeholder_re();
    let mut out = String::with_capacity(s.len());
    let mut last_end = 0;
    for cap in re.captures_iter(s) {
        let m = cap.get(0).unwrap();
        let name = &cap[1];
        let val = std::env::var(name).with_context(|| {
            format!(
                "config field '{}' references ${{{}}} but the env var is not set. \
             Set it in the environment or `.env`, or run under \
             `op run --env-file=secrets.env.1password -- ...` (see README, \"Secrets\")",
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

/// Resolve `${VAR}` placeholders inside the secret-bearing fields of the
/// active profile and the top-level `[surrealdb]` section. Fields outside this
/// set (e.g. ledger URL, token endpoint) are left untouched.
fn resolve_secrets(cfg: &mut Config) -> anyhow::Result<()> {
    if let Some(pw) = cfg.surrealdb.password.as_mut() {
        *pw = substitute_env(pw, "surrealdb.password")?;
    }
    if let Some(kc) = cfg.keycloak.as_mut() {
        match &mut kc.auth_method {
            KeycloakAuthMethod::ClientCredentials { client_secret } => {
                *client_secret = substitute_env(client_secret, "keycloak.client_secret")?;
            }
            KeycloakAuthMethod::Password {
                username,
                password,
                client_secret,
            } => {
                *username = substitute_env(username, "keycloak.username")?;
                *password = substitute_env(password, "keycloak.password")?;
                if let Some(cs) = client_secret.as_mut() {
                    *cs = substitute_env(cs, "keycloak.client_secret")?;
                }
            }
        }
    }
    Ok(())
}

/// A named profile containing environment-specific settings
#[derive(Debug, Deserialize, Clone)]
pub struct ProfileConfig {
    pub ledger: LedgerConfig,
    pub keycloak: Option<KeycloakConfig>,
}

/// Resolved config after selecting a profile
#[derive(Debug)]
pub struct Config {
    pub logging: LoggingConfig,
    pub surrealdb: SurrealConfig,
    pub ledger: LedgerConfig,
    pub keycloak: Option<KeycloakConfig>,
}

/// Authentication method for Keycloak
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "grant_type", rename_all = "snake_case")]
pub enum KeycloakAuthMethod {
    /// OAuth2 Client Credentials flow (service accounts)
    ClientCredentials { client_secret: String },
    /// OAuth2 Resource Owner Password Credentials flow (user authentication)
    Password {
        username: String,
        password: String,
        #[serde(default)]
        client_secret: Option<String>,
    },
}

/// Keycloak OAuth2 configuration
#[derive(Debug, Deserialize, Clone)]
pub struct KeycloakConfig {
    pub client_id: String,
    pub token_endpoint: String,
    #[serde(flatten)]
    pub auth_method: KeycloakAuthMethod,
}

#[derive(Debug, Deserialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

/// `[surrealdb]` section of the config file.
///
/// The connection fields are optional here because each one can instead come
/// from an environment variable (see [`SurrealConfig::resolve`]).
#[derive(Deserialize, Clone, Default)]
pub struct SurrealConfig {
    /// Server URL, e.g. `ws://127.0.0.1:8000`. Env: `SURREAL_URL`.
    pub url: Option<String>,
    /// Env: `SURREAL_NAMESPACE`.
    pub namespace: Option<String>,
    /// Env: `SURREAL_DATABASE`.
    pub database: Option<String>,
    /// A database-level user (never root). Env: `SURREAL_USERNAME`.
    pub username: Option<String>,
    /// Env: `SURREAL_PASSWORD`. May be a `${VAR}` placeholder.
    pub password: Option<String>,
    /// Number of ledger updates written per database transaction
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    /// Flush timeout in seconds - commit even if batch isn't full after this duration
    #[serde(default = "default_flush_timeout")]
    pub flush_timeout_secs: u64,
    /// Idle timeout in seconds - reconnect if no updates received for this duration
    /// Detects dead/stale gRPC streams. Canton sends periodic OffsetCheckpoints,
    /// so a live stream is never silent for long.
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout_secs: u64,
}

impl std::fmt::Debug for SurrealConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurrealConfig")
            .field("url", &self.url)
            .field("namespace", &self.namespace)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("batch_size", &self.batch_size)
            .field("flush_timeout_secs", &self.flush_timeout_secs)
            .field("idle_timeout_secs", &self.idle_timeout_secs)
            .finish()
    }
}

impl SurrealConfig {
    /// Builds the connection settings: each `SURREAL_*` environment variable
    /// overrides the matching config value. Fails if a setting has neither.
    pub fn resolve(&self) -> Result<DbConfig> {
        self.resolve_with(|name| std::env::var(name).ok())
    }

    fn resolve_with(&self, env: impl Fn(&str) -> Option<String>) -> Result<DbConfig> {
        let pick = |var: &str, value: &Option<String>, field: &str| -> Result<String> {
            env(var)
                .filter(|v| !v.is_empty())
                .or_else(|| value.clone())
                .with_context(|| {
                    format!("SurrealDB setting '{field}' is missing: set surrealdb.{field} in the config file or the {var} environment variable")
                })
        };
        Ok(DbConfig {
            url: pick("SURREAL_URL", &self.url, "url")?,
            namespace: pick("SURREAL_NAMESPACE", &self.namespace, "namespace")?,
            database: pick("SURREAL_DATABASE", &self.database, "database")?,
            username: pick("SURREAL_USERNAME", &self.username, "username")?,
            password: pick("SURREAL_PASSWORD", &self.password, "password")?,
        })
    }
}

/// Resolved SurrealDB connection settings.
#[derive(Clone)]
pub struct DbConfig {
    pub url: String,
    pub namespace: String,
    pub database: String,
    pub username: String,
    pub password: String,
}

/// Never prints the password.
impl std::fmt::Debug for DbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConfig")
            .field("url", &self.url)
            .field("namespace", &self.namespace)
            .field("database", &self.database)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Template filter configuration with explicit field names.
/// Uses the package-name reference format with "#" prefix.
#[derive(Debug, Deserialize, Clone)]
pub struct TemplateFilterConfig {
    /// Package name with "#" prefix (e.g., "#splice-amulet")
    pub package_name: String,
    /// Dot-separated module name (e.g., "Splice.Amulet")
    pub module_name: String,
    /// Entity/template name (e.g., "FeaturedAppActivityMarker")
    pub entity_name: String,
}

fn default_batch_size() -> usize {
    100
}

fn default_flush_timeout() -> u64 {
    1
}

fn default_idle_timeout() -> u64 {
    60
}

#[derive(Debug, Deserialize, Clone)]
pub struct LedgerConfig {
    pub fake_jwt_user: String,
    pub parties: Option<Vec<String>>,
    pub url: String,
    /// Optional list of contract templates to filter on.
    pub template_filters: Option<Vec<TemplateFilterConfig>>,
    /// Starting offset for sync when the database has no data.
    /// Positive value: absolute offset. Negative value: relative to ledger end (e.g., -5000000).
    /// If not specified, falls back to ledger pruning offset.
    pub starting_offset: Option<i64>,
}

/// Read and parse the config file
pub fn read_config_file<P: AsRef<Path>>(path: P) -> Result<ConfigFile> {
    let s = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file '{}'", path.as_ref().display()))?;
    let cfg: ConfigFile = toml::from_str(&s).context("failed to parse TOML config")?;
    Ok(cfg)
}

/// Read config file and resolve with the specified profile (or active_profile if None)
pub fn read_config<P: AsRef<Path>>(path: P, profile: Option<&str>) -> Result<Config> {
    let config_file = read_config_file(&path)?;
    resolve_config(config_file, profile)
}

/// Resolve a ConfigFile into a Config using the specified profile
pub fn resolve_config(config_file: ConfigFile, profile_override: Option<&str>) -> Result<Config> {
    let profile_name = profile_override.unwrap_or(&config_file.active_profile);

    let profile = config_file.profiles.get(profile_name).with_context(|| {
        format!(
            "profile '{}' not found. Available profiles: {:?}",
            profile_name,
            config_file.profiles.keys().collect::<Vec<_>>()
        )
    })?;

    let mut cfg = Config {
        logging: config_file.logging,
        surrealdb: config_file.surrealdb,
        ledger: profile.ledger.clone(),
        keycloak: profile.keycloak.clone(),
    };
    resolve_secrets(&mut cfg)?;
    Ok(cfg)
}

pub fn read_config_from_toml(profile: Option<&str>) -> Result<Config> {
    // Try multiple locations for config.toml:
    // 1. ./config/config.toml (relative to current working directory)
    // 2. CARGO_MANIFEST_DIR/config/config.toml (for cargo run)

    let cwd_config = std::path::PathBuf::from("config").join("config.toml");
    if cwd_config.exists() {
        return read_config(&cwd_config, profile);
    }

    if let Ok(crate_root) = std::env::var("CARGO_MANIFEST_DIR") {
        let cargo_config = std::path::PathBuf::from(&crate_root)
            .join("config")
            .join("config.toml");
        if cargo_config.exists() {
            return read_config(&cargo_config, profile);
        }
    }

    anyhow::bail!(
        "Could not find config.toml in ./config/config.toml or CARGO_MANIFEST_DIR/config/config.toml. Use --config-file to specify a path."
    )
}

/// Helper to get available profile names from a config file
pub fn list_profiles<P: AsRef<Path>>(path: P) -> Result<Vec<String>> {
    let config_file = read_config_file(path)?;
    Ok(config_file.profiles.keys().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_env_returns_value_when_set() {
        // SAFETY: env mutation in unit tests; each test uses a unique var name.
        unsafe {
            std::env::set_var("TEST_SUB_ENV_BASIC", "hello");
        }
        let out = substitute_env("${TEST_SUB_ENV_BASIC}", "field.x").unwrap();
        assert_eq!(out, "hello");
        unsafe {
            std::env::remove_var("TEST_SUB_ENV_BASIC");
        }
    }

    #[test]
    fn substitute_env_errors_when_unset() {
        unsafe {
            std::env::remove_var("TEST_SUB_ENV_MISSING");
        }
        let err = substitute_env("${TEST_SUB_ENV_MISSING}", "keycloak.client_secret")
            .expect_err("must fail when env var is unset");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("keycloak.client_secret"),
            "missing field name: {msg}"
        );
        assert!(
            msg.contains("TEST_SUB_ENV_MISSING"),
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
    fn substitute_env_handles_multiple_placeholders() {
        unsafe {
            std::env::set_var("TEST_SUB_MULTI_A", "alpha");
            std::env::set_var("TEST_SUB_MULTI_B", "beta");
        }
        let out = substitute_env("${TEST_SUB_MULTI_A}_${TEST_SUB_MULTI_B}", "x").unwrap();
        assert_eq!(out, "alpha_beta");
        unsafe {
            std::env::remove_var("TEST_SUB_MULTI_A");
            std::env::remove_var("TEST_SUB_MULTI_B");
        }
    }

    fn make_test_config(keycloak: Option<KeycloakConfig>, db_password: &str) -> Config {
        Config {
            logging: LoggingConfig {
                level: "info".to_string(),
            },
            surrealdb: SurrealConfig {
                url: Some("ws://127.0.0.1:8000".to_string()),
                password: Some(db_password.to_string()),
                ..Default::default()
            },
            ledger: LedgerConfig {
                fake_jwt_user: "u".to_string(),
                parties: None,
                url: "http://localhost".to_string(),
                template_filters: None,
                starting_offset: None,
            },
            keycloak,
        }
    }

    #[test]
    fn resolve_secrets_substitutes_db_password() {
        unsafe {
            std::env::set_var("TEST_RS_DB", "secretpw");
        }
        let mut cfg = make_test_config(None, "${TEST_RS_DB}");
        resolve_secrets(&mut cfg).unwrap();
        assert_eq!(cfg.surrealdb.password.as_deref(), Some("secretpw"));
        unsafe {
            std::env::remove_var("TEST_RS_DB");
        }
    }

    #[test]
    fn resolve_secrets_substitutes_client_credentials_secret() {
        unsafe {
            std::env::set_var("TEST_RS_CC", "cc-secret");
            std::env::set_var("TEST_RS_DB2", "n");
        }
        let kc = KeycloakConfig {
            client_id: "id".to_string(),
            token_endpoint: "https://example/token".to_string(),
            auth_method: KeycloakAuthMethod::ClientCredentials {
                client_secret: "${TEST_RS_CC}".to_string(),
            },
        };
        let mut cfg = make_test_config(Some(kc), "${TEST_RS_DB2}");
        resolve_secrets(&mut cfg).unwrap();
        match &cfg.keycloak.as_ref().unwrap().auth_method {
            KeycloakAuthMethod::ClientCredentials { client_secret } => {
                assert_eq!(client_secret, "cc-secret");
            }
            _ => panic!("unexpected auth method"),
        }
        unsafe {
            std::env::remove_var("TEST_RS_CC");
            std::env::remove_var("TEST_RS_DB2");
        }
    }

    #[test]
    fn resolve_secrets_substitutes_password_grant_fields() {
        unsafe {
            std::env::set_var("TEST_RS_USER", "alice");
            std::env::set_var("TEST_RS_PASS", "p4ss");
            std::env::set_var("TEST_RS_CS", "optcs");
            std::env::set_var("TEST_RS_DB3", "n");
        }
        let kc = KeycloakConfig {
            client_id: "id".to_string(),
            token_endpoint: "https://example/token".to_string(),
            auth_method: KeycloakAuthMethod::Password {
                username: "${TEST_RS_USER}".to_string(),
                password: "${TEST_RS_PASS}".to_string(),
                client_secret: Some("${TEST_RS_CS}".to_string()),
            },
        };
        let mut cfg = make_test_config(Some(kc), "${TEST_RS_DB3}");
        resolve_secrets(&mut cfg).unwrap();
        match &cfg.keycloak.as_ref().unwrap().auth_method {
            KeycloakAuthMethod::Password {
                username,
                password,
                client_secret,
            } => {
                assert_eq!(username, "alice");
                assert_eq!(password, "p4ss");
                assert_eq!(client_secret.as_deref(), Some("optcs"));
            }
            _ => panic!("unexpected auth method"),
        }
        unsafe {
            std::env::remove_var("TEST_RS_USER");
            std::env::remove_var("TEST_RS_PASS");
            std::env::remove_var("TEST_RS_CS");
            std::env::remove_var("TEST_RS_DB3");
        }
    }

    #[test]
    fn resolve_secrets_errors_with_actionable_message() {
        unsafe {
            std::env::remove_var("TEST_RS_MISSING");
            std::env::set_var("TEST_RS_DB_OK", "n");
        }
        let kc = KeycloakConfig {
            client_id: "id".to_string(),
            token_endpoint: "https://example/token".to_string(),
            auth_method: KeycloakAuthMethod::ClientCredentials {
                client_secret: "${TEST_RS_MISSING}".to_string(),
            },
        };
        let mut cfg = make_test_config(Some(kc), "${TEST_RS_DB_OK}");
        let err = resolve_secrets(&mut cfg).expect_err("must fail when env var unset");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("keycloak.client_secret"),
            "missing field: {msg}"
        );
        assert!(msg.contains("TEST_RS_MISSING"), "missing var: {msg}");
        assert!(
            msg.contains("op run --env-file=secrets.env.1password"),
            "missing op run guidance: {msg}"
        );
        unsafe {
            std::env::remove_var("TEST_RS_DB_OK");
        }
    }

    #[test]
    fn resolve_config_runs_secret_substitution() {
        unsafe {
            std::env::set_var("TEST_RC_DB", "db-pw");
            std::env::set_var("TEST_RC_CS", "kc-secret");
        }

        let toml_src = r#"
active_profile = "p"

[logging]
level = "info"

[surrealdb]
url = "ws://127.0.0.1:8000"
password = "${TEST_RC_DB}"

[profiles.p.ledger]
fake_jwt_user = "u"
url = "http://localhost"

[profiles.p.keycloak]
client_id = "cid"
token_endpoint = "https://example/token"
grant_type = "client_credentials"
client_secret = "${TEST_RC_CS}"
"#;

        let config_file: ConfigFile = toml::from_str(toml_src).unwrap();
        let cfg = resolve_config(config_file, None).unwrap();

        assert_eq!(cfg.surrealdb.password.as_deref(), Some("db-pw"));
        match &cfg.keycloak.as_ref().unwrap().auth_method {
            KeycloakAuthMethod::ClientCredentials { client_secret } => {
                assert_eq!(client_secret, "kc-secret");
            }
            _ => panic!("unexpected auth method"),
        }

        unsafe {
            std::env::remove_var("TEST_RC_DB");
            std::env::remove_var("TEST_RC_CS");
        }
    }

    #[test]
    fn resolve_config_does_not_visit_inactive_profiles() {
        // Only INACTIVE_VAR is intentionally NOT set — if the loader were
        // eagerly substituting, this would fail. resolve_config discards
        // inactive profiles before resolve_secrets runs, so it must succeed.
        unsafe {
            std::env::remove_var("RC_INACTIVE_VAR_DO_NOT_SET");
            std::env::set_var("RC_ACTIVE_CS", "active-cs");
            std::env::set_var("RC_ACTIVE_DB", "n");
        }

        let toml_src = r#"
active_profile = "active"

[logging]
level = "info"

[surrealdb]
url = "ws://127.0.0.1:8000"
password = "${RC_ACTIVE_DB}"

[profiles.active.ledger]
fake_jwt_user = "u"
url = "http://localhost"

[profiles.active.keycloak]
client_id = "cid"
token_endpoint = "https://example/token"
grant_type = "client_credentials"
client_secret = "${RC_ACTIVE_CS}"

[profiles.inactive.ledger]
fake_jwt_user = "u"
url = "http://localhost"

[profiles.inactive.keycloak]
client_id = "cid"
token_endpoint = "https://example/token"
grant_type = "client_credentials"
client_secret = "${RC_INACTIVE_VAR_DO_NOT_SET}"
"#;

        let config_file: ConfigFile = toml::from_str(toml_src).unwrap();
        let cfg = resolve_config(config_file, None)
            .expect("lazy substitution: inactive profile's unresolved ${VAR} must not error");
        assert_eq!(cfg.surrealdb.password.as_deref(), Some("n"));

        unsafe {
            std::env::remove_var("RC_ACTIVE_CS");
            std::env::remove_var("RC_ACTIVE_DB");
        }
    }

    fn full_surreal_config() -> SurrealConfig {
        SurrealConfig {
            url: Some("ws://from-file:8000".into()),
            namespace: Some("ns".into()),
            database: Some("db".into()),
            username: Some("user".into()),
            password: Some("file-pw".into()),
            ..Default::default()
        }
    }

    #[test]
    fn surreal_env_overrides_config_file() {
        let env = |name: &str| match name {
            "SURREAL_URL" => Some("ws://from-env:9000".to_string()),
            "SURREAL_PASSWORD" => Some("env-pw".to_string()),
            _ => None,
        };
        let db = full_surreal_config().resolve_with(env).unwrap();
        assert_eq!(db.url, "ws://from-env:9000");
        assert_eq!(db.password, "env-pw");
        assert_eq!(db.namespace, "ns");
        assert_eq!(db.username, "user");
    }

    #[test]
    fn surreal_missing_setting_names_env_var() {
        let mut cfg = full_surreal_config();
        cfg.database = None;
        let err = cfg.resolve_with(|_| None).unwrap_err();
        assert!(format!("{err:#}").contains("SURREAL_DATABASE"));
    }

    #[test]
    fn surreal_debug_never_prints_password() {
        let db = full_surreal_config().resolve_with(|_| None).unwrap();
        let printed = format!("{db:?} {:?}", full_surreal_config());
        assert!(!printed.contains("file-pw"), "{printed}");
    }
}
