//! Runs the `syncer` binary as a child process and turns its life into a
//! stream of [`Event`]s: started, one event per output line, exited.
//!
//! The UI never runs the sync in its own process. Stopping it from the window
//! therefore behaves like `just sync-stop`: the process gets a termination
//! signal and resumes from the last stored offset on the next start.

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use iced::futures::{SinkExt, Stream, channel::mpsc::Sender};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader, Lines};
use tokio::process::Command;
use tokio::sync::mpsc;

/// How long a stopping syncer gets to exit on its own before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(10);

/// What to launch.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    /// Arguments before the syncer's own (e.g. `run --release -p syncer --`).
    pub prefix_args: Vec<String>,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// Every output line is also written here, like `tee sync.log`.
    pub log_file: PathBuf,
}

/// A clonable handle that asks the running syncer to stop.
#[derive(Debug, Clone)]
pub struct StopHandle(mpsc::UnboundedSender<()>);

impl StopHandle {
    pub fn stop(&self) {
        let _ = self.0.send(());
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Started {
        pid: Option<u32>,
        stop: StopHandle,
    },
    Line(String),
    /// The process ended. `ok` is true for a zero exit code.
    Exited {
        ok: bool,
        summary: String,
    },
}

/// Where the `syncer` program is: `SYNCER_BIN`, then next to this program
/// (both live in `target/<profile>/`), else build and run it with cargo from
/// the workspace root.
pub fn locate_syncer() -> (PathBuf, Vec<String>) {
    if let Some(bin) = std::env::var_os("SYNCER_BIN").filter(|v| !v.is_empty()) {
        return (PathBuf::from(bin), Vec::new());
    }
    let name = format!("syncer{}", std::env::consts::EXE_SUFFIX);
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|p| p.is_file())
    {
        return (sibling, Vec::new());
    }
    (
        PathBuf::from("cargo"),
        ["run", "--release", "-p", "syncer", "--"]
            .map(String::from)
            .to_vec(),
    )
}

pub fn run(launch: Launch) -> impl Stream<Item = Event> {
    iced::stream::channel(256, async move |mut out: Sender<Event>| {
        let summary = supervise(launch, &mut out).await;
        let _ = out.send(summary).await;
    })
}

async fn supervise(launch: Launch, out: &mut Sender<Event>) -> Event {
    let mut log = match File::create(&launch.log_file) {
        Ok(f) => Some(f),
        Err(e) => {
            let _ = out
                .send(Event::Line(format!(
                    "(could not write {}: {e})",
                    launch.log_file.display()
                )))
                .await;
            None
        }
    };

    let mut command = Command::new(&launch.program);
    command
        .args(&launch.prefix_args)
        .args(&launch.args)
        .current_dir(&launch.cwd)
        // Plain text in the window and in sync.log.
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // If the window goes away, the syncer goes with it.
        .kill_on_drop(true);

    let shown = std::iter::once(launch.program.display().to_string())
        .chain(launch.prefix_args.iter().cloned())
        .chain(launch.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    emit(out, &mut log, format!("$ {shown}")).await;

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            return Event::Exited {
                ok: false,
                summary: format!("could not start {}: {e}", launch.program.display()),
            };
        }
    };

    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel();
    let pid = child.id();
    let _ = out
        .send(Event::Started {
            pid,
            stop: StopHandle(stop_tx),
        })
        .await;

    let mut stdout = child.stdout.take().map(|s| BufReader::new(s).lines());
    let mut stderr = child.stderr.take().map(|s| BufReader::new(s).lines());

    let mut stopping = false;
    let mut stop_closed = false;
    let mut force_kill = false;
    let mut deadline: Option<tokio::time::Instant> = None;

    let status = loop {
        tokio::select! {
            line = next_line(&mut stdout) => emit_opt(out, &mut log, &mut stdout, line).await,
            line = next_line(&mut stderr) => emit_opt(out, &mut log, &mut stderr, line).await,
            req = stop_rx.recv(), if !stopping && !stop_closed => match req {
                Some(()) => {
                    stopping = true;
                    if terminate(pid) {
                        emit(out, &mut log, "(stop requested, waiting for the syncer to exit)".into()).await;
                        deadline = Some(tokio::time::Instant::now() + STOP_GRACE);
                    } else {
                        force_kill = true;
                    }
                }
                None => stop_closed = true,
            },
            _ = sleep_until(deadline) => {
                emit(out, &mut log, "(the syncer did not exit in time; killing it)".into()).await;
                force_kill = true;
            }
            status = child.wait() => break status,
        }
        if force_kill {
            let _ = child.kill().await;
            break child.wait().await;
        }
    };

    // Whatever is still buffered in the pipes.
    let drain = async {
        while let Some(line) = next_line(&mut stdout).await {
            emit(out, &mut log, line).await;
        }
        while let Some(line) = next_line(&mut stderr).await {
            emit(out, &mut log, line).await;
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), drain).await;

    match status {
        Ok(s) if s.success() => Event::Exited {
            ok: true,
            summary: "exited normally".into(),
        },
        Ok(_) if stopping => Event::Exited {
            ok: true,
            summary: "stopped".into(),
        },
        Ok(s) => Event::Exited {
            ok: false,
            summary: match s.code() {
                Some(code) => format!("exited with code {code}"),
                None => "ended by a signal".into(),
            },
        },
        Err(e) => Event::Exited {
            ok: false,
            summary: format!("lost track of the process: {e}"),
        },
    }
}

/// Polite stop: SIGTERM on Unix (what `pkill` sends). Elsewhere there is no
/// polite signal, so the caller kills the process.
#[cfg(unix)]
fn terminate(pid: Option<u32>) -> bool {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;
    match pid.and_then(|p| i32::try_from(p).ok()) {
        Some(p) => kill(Pid::from_raw(p), Signal::SIGTERM).is_ok(),
        None => false,
    }
}

#[cfg(not(unix))]
fn terminate(_pid: Option<u32>) -> bool {
    false
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending().await,
    }
}

/// The next line of a pipe, or pending forever once it is closed.
async fn next_line<R: AsyncRead + Unpin>(
    lines: &mut Option<Lines<BufReader<R>>>,
) -> Option<String> {
    match lines {
        Some(l) => l.next_line().await.ok().flatten(),
        None => std::future::pending().await,
    }
}

async fn emit_opt<R>(
    out: &mut Sender<Event>,
    log: &mut Option<File>,
    pipe: &mut Option<Lines<BufReader<R>>>,
    line: Option<String>,
) {
    match line {
        Some(line) => emit(out, log, line).await,
        None => *pipe = None,
    }
}

async fn emit(out: &mut Sender<Event>, log: &mut Option<File>, line: String) {
    let line = strip_ansi(&line);
    if let Some(f) = log.as_mut() {
        let _ = writeln!(f, "{line}");
    }
    let _ = out.send(Event::Line(line)).await;
}

/// Drop terminal colour codes (`ESC [ ... letter`) in case the child ignores
/// `NO_COLOR`.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.clone().next() == Some('[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(
            strip_ansi("\u{1b}[2m2026\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m hi"),
            "2026  INFO hi"
        );
        assert_eq!(strip_ansi("plain"), "plain");
    }
}
