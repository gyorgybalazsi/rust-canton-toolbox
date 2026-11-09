use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

/// Sync process status
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub profile: Option<String>,
    pub fresh: bool,
    pub log_lines: Vec<String>,
    pub neo4j_offset: Option<i64>,
    pub transaction_count: Option<u64>,
}

/// Available sync profiles from config
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncProfile {
    pub name: String,
    pub has_keycloak: bool,
    pub url: String,
    pub starting_offset: Option<i64>,
}

#[cfg(feature = "server")]
mod process {
    use std::sync::Mutex;
    use tokio::process::Child;

    pub struct SyncProcess {
        pub child: Child,
        pub profile: String,
        pub fresh: bool,
        pub log_lines: Mutex<Vec<String>>,
    }

    static SYNC_PROCESS: std::sync::OnceLock<Mutex<Option<SyncProcess>>> =
        std::sync::OnceLock::new();

    pub fn state() -> &'static Mutex<Option<SyncProcess>> {
        SYNC_PROCESS.get_or_init(|| Mutex::new(None))
    }
}

/// Get available sync profiles from config
#[server]
pub async fn get_sync_profiles() -> Result<Vec<SyncProfile>, ServerFnError> {
    // Read the config file and extract profile names
    let config_path = find_explorer_config()
        .ok_or_else(|| ServerFnError::new("Could not find event-graph-indexer config.toml"))?;

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| ServerFnError::new(format!("Failed to read config: {e}")))?;

    // Parse as generic TOML to extract profile names
    let table: toml::Value = toml::from_str(&content)
        .map_err(|e| ServerFnError::new(format!("Failed to parse config: {e}")))?;

    let mut profiles = Vec::new();
    if let Some(toml::Value::Table(profs)) = table.get("profiles") {
        for (name, value) in profs {
            let has_keycloak = value.get("keycloak").is_some();
            let url = value
                .get("ledger")
                .and_then(|l| l.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("unknown")
                .to_string();
            let starting_offset = value
                .get("ledger")
                .and_then(|l| l.get("starting_offset"))
                .and_then(|v| v.as_integer());
            profiles.push(SyncProfile {
                name: name.clone(),
                has_keycloak,
                url,
                starting_offset,
            });
        }
    }

    // Sort: local first, then devnet, then mainnet, then alphabetical
    let order = |name: &str| -> usize {
        match name {
            "local" => 0,
            "devnet" => 1,
            "mainnet" => 2,
            _ => 3,
        }
    };
    profiles.sort_by(|a, b| {
        order(&a.name)
            .cmp(&order(&b.name))
            .then(a.name.cmp(&b.name))
    });
    Ok(profiles)
}

/// Start the sync process
#[server]
pub async fn start_sync(
    profile: String,
    fresh: bool,
    starting_offset: Option<i64>,
) -> Result<(), ServerFnError> {
    use std::process::Stdio;
    use tokio::process::Command;

    let state = process::state();
    let mut guard = state.lock().unwrap();

    // Check if already running
    if let Some(ref mut proc) = *guard {
        match proc.child.try_wait() {
            Ok(Some(_)) => {} // exited, we can start new
            Ok(None) => return Err(ServerFnError::new("Sync is already running")),
            Err(_) => {}
        }
    }

    // Find the explorer binary
    let binary = find_explorer_binary()
        .ok_or_else(|| ServerFnError::new("Could not find event-graph-indexer binary. Run 'cargo build --release -p event-graph-indexer' first."))?;

    let config_path = find_explorer_config()
        .ok_or_else(|| ServerFnError::new("Could not find event-graph-indexer config.toml"))?;

    // Read and optionally modify config
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| ServerFnError::new(format!("Failed to read config: {e}")))?;
    let mut table: toml::Value = toml::from_str(&content)
        .map_err(|e| ServerFnError::new(format!("Failed to parse config: {e}")))?;
    let has_keycloak = table
        .get("profiles")
        .and_then(|p| p.get(&profile))
        .and_then(|p| p.get("keycloak"))
        .is_some();

    // If starting_offset specified, write a temp config with the override
    let effective_config_path = if let Some(offset) = starting_offset {
        if let Some(ledger) = table
            .get_mut("profiles")
            .and_then(|p| p.get_mut(&profile))
            .and_then(|p| p.get_mut("ledger"))
        {
            ledger.as_table_mut().map(|t| {
                t.insert("starting_offset".to_string(), toml::Value::Integer(offset));
            });
        }
        let tmp_path = "/tmp/event-graph-ui-sync-config.toml";
        let modified = toml::to_string_pretty(&table)
            .map_err(|e| ServerFnError::new(format!("Failed to serialize config: {e}")))?;
        std::fs::write(tmp_path, modified)
            .map_err(|e| ServerFnError::new(format!("Failed to write temp config: {e}")))?;
        tmp_path.to_string()
    } else {
        config_path.clone()
    };

    let mut cmd = Command::new(&binary);
    cmd.arg("sync")
        .arg("--config-file")
        .arg(&effective_config_path)
        .arg("--profile")
        .arg(&profile);

    if has_keycloak {
        cmd.arg("--use-keycloak");
    }

    if fresh {
        cmd.arg("--fresh");
    }

    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| ServerFnError::new(format!("Failed to start sync: {e}")))?;

    let pid = child.id();
    tracing::info!("Sync started: profile={profile}, fresh={fresh}, pid={pid:?}");

    // Spawn a task to read stderr and collect log lines
    let stderr = child.stderr.take();
    let stdout = child.stdout.take();

    let proc = process::SyncProcess {
        child,
        profile: profile.clone(),
        fresh,
        log_lines: std::sync::Mutex::new(Vec::new()),
    };

    *guard = Some(proc);
    drop(guard);

    // Background task to read output
    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let state = process::state();
                if let Ok(guard) = state.lock() {
                    if let Some(ref proc) = *guard {
                        if let Ok(mut log) = proc.log_lines.lock() {
                            log.push(line);
                            // Keep last 200 lines
                            if log.len() > 200 {
                                let drain = log.len() - 200;
                                log.drain(..drain);
                            }
                        }
                    }
                }
            }
        });
    }

    if let Some(stdout) = stdout {
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let state = process::state();
                if let Ok(guard) = state.lock() {
                    if let Some(ref proc) = *guard {
                        if let Ok(mut log) = proc.log_lines.lock() {
                            log.push(line);
                            if log.len() > 200 {
                                let drain = log.len() - 200;
                                log.drain(..drain);
                            }
                        }
                    }
                }
            }
        });
    }

    Ok(())
}

/// Stop the sync process
#[server]
pub async fn stop_sync() -> Result<(), ServerFnError> {
    let state = process::state();
    let mut guard = state.lock().unwrap();

    if let Some(ref mut proc) = *guard {
        match proc.child.try_wait() {
            Ok(Some(_)) => {
                // Already exited
                *guard = None;
                return Ok(());
            }
            Ok(None) => {
                // Still running, kill it
                proc.child
                    .kill()
                    .await
                    .map_err(|e| ServerFnError::new(format!("Failed to kill sync process: {e}")))?;
                *guard = None;
                tracing::info!("Sync process stopped");
                return Ok(());
            }
            Err(e) => {
                *guard = None;
                return Err(ServerFnError::new(format!("Error checking process: {e}")));
            }
        }
    }

    Err(ServerFnError::new("No sync process is running"))
}

/// Get current sync status
#[server]
pub async fn get_sync_status() -> Result<SyncStatus, ServerFnError> {
    let pool = super::neo4j_pool::pool();

    // Check process state
    let (running, pid, profile, fresh, log_lines) = {
        let state = process::state();
        let mut guard = state.lock().unwrap();

        if let Some(ref mut proc) = *guard {
            let is_running = match proc.child.try_wait() {
                Ok(Some(_)) => false, // exited
                Ok(None) => true,     // still running
                Err(_) => false,
            };
            let pid = proc.child.id();
            let profile = proc.profile.clone();
            let fresh = proc.fresh;
            let lines = proc.log_lines.lock().map(|l| l.clone()).unwrap_or_default();

            if !is_running {
                // Process exited, clean up but keep logs
                let lines_copy = lines.clone();
                *guard = None;
                (false, pid, Some(profile), fresh, lines_copy)
            } else {
                (true, pid, Some(profile), fresh, lines)
            }
        } else {
            (false, None, None, false, Vec::new())
        }
    };

    // Query Neo4j for current offset and transaction count
    let neo4j_offset = {
        let q = neo4rs::query("MATCH (t:Transaction) RETURN max(t.offset) AS max_off");
        let mut result = pool
            .execute(q)
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j query failed: {e}")))?;
        if let Some(row) = result
            .next()
            .await
            .map_err(|e| ServerFnError::new(format!("Failed to read row: {e}")))?
        {
            row.get::<i64>("max_off").ok()
        } else {
            None
        }
    };

    let transaction_count = {
        let q = neo4rs::query("MATCH (t:Transaction) RETURN count(t) AS cnt");
        let mut result = pool
            .execute(q)
            .await
            .map_err(|e| ServerFnError::new(format!("Neo4j query failed: {e}")))?;
        if let Some(row) = result
            .next()
            .await
            .map_err(|e| ServerFnError::new(format!("Failed to read row: {e}")))?
        {
            row.get::<i64>("cnt").ok().map(|c| c as u64)
        } else {
            None
        }
    };

    Ok(SyncStatus {
        running,
        pid,
        profile,
        fresh,
        log_lines,
        neo4j_offset,
        transaction_count,
    })
}

#[cfg(feature = "server")]
fn find_explorer_binary() -> Option<String> {
    // Check common locations
    let candidates = [
        "target/release/event-graph-indexer",
        "../target/release/event-graph-indexer",
        "target/debug/event-graph-indexer",
        "../target/debug/event-graph-indexer",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    // Check PATH
    if let Ok(output) = std::process::Command::new("which")
        .arg("event-graph-indexer")
        .output()
    {
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).trim().to_string());
        }
    }
    None
}

#[cfg(feature = "server")]
fn find_explorer_config() -> Option<String> {
    let candidates = [
        "config.toml",
        "config/config.toml",
        "../event-graph-indexer/config/config.toml",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            return Some(path.to_string());
        }
    }
    None
}
