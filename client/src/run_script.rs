use anyhow::Result;
use std::path::Path;
use std::process::{Command, Output};
use tracing::{error, info};

/// Runs a Daml script on a ledger using the `dpm script` CLI command.
///
/// # Arguments
/// * `ledger_host` - The ledger host (e.g., "localhost")
/// * `ledger_port` - The ledger port (e.g., 6865)
/// * `dar_path` - Path to the DAR file containing the script
/// * `script_name` - The fully qualified script name (e.g., "Setup:setup")
///
/// # Returns
/// * `Ok(String)` - The stdout output from the script execution
/// * `Err` - If the script execution fails
pub fn run_script(
    ledger_host: &str,
    ledger_port: u16,
    dar_path: &Path,
    script_name: &str,
) -> Result<String> {
    info!(
        "Running Daml script '{}' from DAR {:?} on {}:{}",
        script_name, dar_path, ledger_host, ledger_port
    );

    let output: Output = Command::new("dpm")
        .args([
            "script",
            "--ledger-host",
            ledger_host,
            "--ledger-port",
            &ledger_port.to_string(),
            "--dar",
            dar_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid DAR path"))?,
            "--script-name",
            script_name,
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run dpm script: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        error!(
            "dpm script failed with exit code: {:?}",
            output.status.code()
        );
        error!("stdout: {}", stdout);
        error!("stderr: {}", stderr);
        return Err(anyhow::anyhow!(
            "dpm script '{}' failed: {}",
            script_name,
            stderr
        ));
    }

    info!("Script '{}' completed successfully", script_name);
    if !stdout.is_empty() {
        info!("Script output: {}", stdout);
    }

    Ok(stdout)
}

/// Runs a Daml script with a working directory context.
/// This is useful when the script needs to be run from a specific directory.
///
/// # Arguments
/// * `working_dir` - The working directory to run the script from
/// * `ledger_host` - The ledger host (e.g., "localhost")
/// * `ledger_port` - The ledger port (e.g., 6865)
/// * `dar_path` - Path to the DAR file containing the script
/// * `script_name` - The fully qualified script name (e.g., "Setup:setup")
///
/// # Returns
/// * `Ok(String)` - The stdout output from the script execution
/// * `Err` - If the script execution fails
pub fn run_script_in_dir(
    working_dir: &Path,
    ledger_host: &str,
    ledger_port: u16,
    dar_path: &Path,
    script_name: &str,
) -> Result<String> {
    info!(
        "Running Daml script '{}' from DAR {:?} on {}:{} in directory {:?}",
        script_name, dar_path, ledger_host, ledger_port, working_dir
    );

    let output: Output = Command::new("dpm")
        .args([
            "script",
            "--ledger-host",
            ledger_host,
            "--ledger-port",
            &ledger_port.to_string(),
            "--dar",
            dar_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid DAR path"))?,
            "--script-name",
            script_name,
        ])
        .current_dir(working_dir)
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run dpm script: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        error!(
            "dpm script failed with exit code: {:?}",
            output.status.code()
        );
        error!("stdout: {}", stdout);
        error!("stderr: {}", stderr);
        return Err(anyhow::anyhow!(
            "dpm script '{}' failed: {}",
            script_name,
            stderr
        ));
    }

    info!("Script '{}' completed successfully", script_name);
    if !stdout.is_empty() {
        info!("Script output: {}", stdout);
    }

    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutils::start_sandbox;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_run_script() -> Result<()> {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let crate_root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let script_dir = PathBuf::from(&crate_root)
            .join("..")
            .join("_daml")
            .join("daml-ticketoffer-explicit-disclosure");

        let main_dir = script_dir.join("main");
        let test_dir = script_dir.join("test");
        let main_dar = main_dir
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-explicit-disclosure-0.0.1.dar");
        let test_dar = test_dir
            .join(".daml")
            .join("dist")
            .join("daml-ticketoffer-explicit-disclosure-test-0.0.1.dar");

        let sandbox_port = 6865;
        let _guard = start_sandbox(main_dir, main_dar, sandbox_port)
            .await
            .expect("Failed to start sandbox");

        let result = run_script("localhost", sandbox_port, &test_dar, "Setup:setup")?;

        info!("Script result: {}", result);
        Ok(())
    }
}
