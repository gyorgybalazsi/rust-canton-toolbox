use anyhow::Result;
use clap::Parser;
use nix::libc;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use std::io::{BufRead, BufReader};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "sandbox-init")]
#[command(about = "Starts a Daml sandbox and runs an initialization script")]
struct Cli {
    /// Path to the Daml model DAR file
    #[arg(long)]
    dar: PathBuf,

    /// Path to the DAR file containing the init script
    #[arg(long)]
    init_dar: PathBuf,

    /// Identifier of the init script (format: Module.Name:Entity.Name)
    #[arg(long)]
    init_script_name: String,
}

struct SandboxGuard {
    child: Option<Child>,
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = close_sandbox(child);
        }
    }
}

fn close_sandbox(child: &mut Child) -> Result<()> {
    let pgid = child.id();
    killpg(Pid::from_raw(pgid as i32), Signal::SIGKILL)
        .map_err(|e| anyhow::anyhow!("Failed to send SIGKILL to sandbox process group: {}", e))?;
    child
        .wait()
        .map_err(|e| anyhow::anyhow!("Failed to wait for sandbox to exit: {}", e))?;
    Ok(())
}

fn start_sandbox(dar_path: &PathBuf) -> Result<SandboxGuard> {
    info!("Starting sandbox with DAR: {:?}", dar_path);

    let mut child;
    unsafe {
        child = Command::new("dpm")
            .args(["sandbox", "--dar", dar_path.to_str().unwrap()])
            .stdout(Stdio::piped())
            .pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            })
            .spawn()
            .map_err(|e| anyhow::anyhow!("Failed to start sandbox: {}", e))?;
    }

    wait_for_sandbox_ready(&mut child)?;

    Ok(SandboxGuard { child: Some(child) })
}

fn wait_for_sandbox_ready(child: &mut Child) -> Result<()> {
    let stdout = child
        .stdout
        .take()
        .expect("Failed to capture sandbox stdout");
    info!("Waiting for sandbox to be ready...");
    let reader = BufReader::new(stdout);

    for line in reader.lines().take(120) {
        let line = line?;
        info!("Sandbox: {}", line);
        if line.contains("Canton sandbox is ready") {
            info!("Sandbox is ready!");
            return Ok(());
        }
    }
    Err(anyhow::anyhow!(
        "Sandbox did not print readiness message in time"
    ))
}

fn run_init_script(init_dar: &PathBuf, init_script_name: &str) -> Result<()> {
    info!(
        "Running init script '{}' from DAR {:?}",
        init_script_name, init_dar
    );

    let output = Command::new("dpm")
        .args([
            "script",
            "--dar",
            init_dar.to_str().unwrap(),
            "--script-name",
            init_script_name,
            "--ledger-host",
            "localhost",
            "--ledger-port",
            "6865",
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run dpm script: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Init script '{}' failed:\nstdout: {}\nstderr: {}",
            init_script_name,
            stdout,
            stderr
        ));
    }

    info!("Init script completed successfully");
    if !stdout.is_empty() {
        info!("Script output: {}", stdout);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let cli = Cli::parse();

    info!("Starting sandbox-init");
    info!("  DAR: {:?}", cli.dar);
    info!("  Init DAR: {:?}", cli.init_dar);
    info!("  Init script: {}", cli.init_script_name);

    let _guard = start_sandbox(&cli.dar)?;

    run_init_script(&cli.init_dar, &cli.init_script_name)?;

    info!("Sandbox initialized successfully. Press Ctrl+C to stop.");

    // Keep running until interrupted
    tokio::signal::ctrl_c().await?;
    info!("Shutting down...");

    Ok(())
}
