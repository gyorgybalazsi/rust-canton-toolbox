//! Reads `config.toml` the way the syncer will, so problems (missing file,
//! unknown profile, unset secret) show up in the window before anything starts.
//! Secret values are never shown or kept: only whether they resolved.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use syncer::config::{self, KeycloakAuthMethod};

/// The workspace root: where `justfile`, `.env` and `sync.log` live.
pub fn workspace_root() -> PathBuf {
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf);
    match compiled {
        Some(root) if root.join("syncer").is_dir() => root,
        _ => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

pub fn default_config_path(root: &Path) -> PathBuf {
    root.join("syncer").join("config").join("config.toml")
}

pub fn example_config_path(root: &Path) -> PathBuf {
    root.join("syncer")
        .join("config")
        .join("config.toml.example")
}

/// Reads `.env` from the workspace root into the process environment, like
/// `just` does with `dotenv-load`. Variables that were already set when the
/// window opened always win; the others follow the file, so editing `.env`
/// and pressing Reload picks up the change.
pub struct DotEnv {
    path: PathBuf,
    preset: HashSet<OsString>,
}

impl DotEnv {
    pub fn new(root: &Path) -> Self {
        DotEnv {
            path: root.join(".env"),
            preset: std::env::vars_os().map(|(k, _)| k).collect(),
        }
    }

    /// Applies the file and returns a line for the UI. Values are never shown.
    pub fn load(&self) -> String {
        if !self.path.is_file() {
            return "Secrets come from the environment (no .env file at the repository root)."
                .into();
        }
        let items = match dotenvy::from_path_iter(&self.path) {
            Ok(items) => items,
            Err(e) => return format!("Could not read {}: {e}", self.path.display()),
        };
        for item in items {
            match item {
                Ok((key, value)) if !self.preset.contains(OsStr::new(&key)) => {
                    // SAFETY: only called from the UI thread, between runs,
                    // when nothing else in this program reads the environment.
                    unsafe { std::env::set_var(key, value) };
                }
                Ok(_) => {}
                Err(e) => return format!("Could not read {}: {e}", self.path.display()),
            }
        }
        format!(
            "Secrets come from the environment and {}.",
            self.path.display()
        )
    }
}

/// The profiles in a config file, sorted, with the file's `active_profile`.
pub struct ConfigProfiles {
    pub profiles: Vec<String>,
    pub active: String,
}

pub fn read_profiles(path: &Path) -> Result<ConfigProfiles, String> {
    if !path.is_file() {
        return Err(format!("{} does not exist yet.", path.display()));
    }
    let file = config::read_config_file(path).map_err(|e| format!("{e:#}"))?;
    let mut profiles: Vec<String> = file.profiles.keys().cloned().collect();
    profiles.sort();
    Ok(ConfigProfiles {
        profiles,
        active: file.active_profile,
    })
}

/// What a profile will do, in words a newcomer can check.
#[derive(Debug, Clone)]
pub struct Ready {
    pub ledger_url: String,
    pub sign_in: String,
    pub use_keycloak: bool,
    pub parties: String,
    pub database: String,
    /// The data a fresh start deletes, for the confirmation.
    pub fresh_target: String,
}

pub fn check_profile(path: &Path, profile: &str) -> Result<Ready, String> {
    let cfg = config::read_config(path, Some(profile)).map_err(|e| format!("{e:#}"))?;
    let db = cfg.surrealdb.resolve().map_err(|e| format!("{e:#}"))?;

    let (sign_in, use_keycloak) = match &cfg.keycloak {
        Some(kc) => {
            let flow = match kc.auth_method {
                KeycloakAuthMethod::ClientCredentials { .. } => "client credentials",
                KeycloakAuthMethod::Password { .. } => "user and password",
            };
            (
                format!("Keycloak ({flow}) as client '{}'", kc.client_id),
                true,
            )
        }
        None => (
            format!(
                "Unsigned development token for user '{}' (local sandbox only)",
                cfg.ledger.fake_jwt_user
            ),
            false,
        ),
    };

    let parties = match cfg.ledger.parties.as_deref() {
        None | Some([]) => "All parties known to the ledger, discovered at start".to_string(),
        Some([one]) => one.clone(),
        Some(many) => format!("{} configured parties", many.len()),
    };

    Ok(Ready {
        ledger_url: cfg.ledger.url,
        sign_in,
        use_keycloak,
        parties,
        database: format!(
            "{} at {}, namespace '{}', database '{}'",
            db.username, db.url, db.namespace, db.database
        ),
        fresh_target: format!(
            "database '{}' (namespace '{}') at {}",
            db.database, db.namespace, db.url
        ),
    })
}
